//! Runs Playdate C games: the device build (`pdex.elf`, ARM Thumb-2) runs in
//! a Unicorn Cortex-M7 emulator. Each basic block is decoded once with
//! Capstone and costed with the calibrated cost of each instruction class;
//! API calls are intercepted and run on the simulated device.

pub mod abi;
pub mod api;
pub mod cost;
pub mod elf;
pub mod emu;
pub mod heap;
pub mod layout;

pub use emu::ArmGame;

#[cfg(test)]
mod emulator_sanity {
    use unicorn_engine::unicorn_const::{Arch, Mode, Prot};
    use unicorn_engine::{ArmCpuModel, RegisterARM, Unicorn};

    #[test]
    /// Cortex-M7 FPU instructions run, and a hook can call back into
    /// emulated code (nested emu_start) and return by setting PC = LR.
    fn fpu_and_nested_calls() {
        let mut uc = Unicorn::new_with_data(Arch::ARM, Mode::THUMB | Mode::MCLASS, 0u32).unwrap();
        uc.ctl_set_cpu_model(ArmCpuModel::CORTEX_M7.into()).unwrap();
        uc.mem_map(0x1000_0000, 0x10000, Prot::ALL).unwrap();
        uc.mem_map(0x2000_0000, 0x10000, Prot::ALL).unwrap();
        // main: vmov s0, r0 ; vadd.f32 s0, s0, s0 ; vmov r0, s0 ; push {lr}; blx r4 ; pop {pc}
        // callee at 0x1000_0100: bx lr (hooked; nested call into 0x1000_0200)
        // nested at 0x1000_0200: adds r0, #1 ; bx lr
        let main: [u8; 16] = [
            0x00, 0xee, 0x10, 0x0a, // vmov s0, r0
            0x30, 0xee, 0x00, 0x0a, // vadd.f32 s0, s0, s0
            0x10, 0xee, 0x10, 0x0a, // vmov r0, s0
            0xa0, 0x47, // blx r4
            0x00, 0xbf, // nop
        ];
        uc.mem_write(0x1000_0000, &main).unwrap();
        uc.mem_write(0x1000_0100, &[0x70, 0x47]).unwrap();
        uc.mem_write(0x1000_0200, &[0x01, 0x30, 0x70, 0x47]).unwrap();
        uc.mem_write(0x1000_0300, &[0x00, 0xbf]).unwrap();
        uc.reg_write(RegisterARM::SP, 0x2000_8000).unwrap();
        uc.reg_write(RegisterARM::R4, 0x1000_0101).unwrap();
        uc.add_code_hook(0x1000_0100, 0x1000_0101, |uc, _, _| {
            let lr = uc.reg_read(RegisterARM::LR).unwrap();
            let r0 = uc.reg_read(RegisterARM::R0).unwrap();
            uc.reg_write(RegisterARM::R0, 41).unwrap();
            uc.reg_write(RegisterARM::LR, 0x1000_0301).unwrap();
            uc.emu_start(0x1000_0201, 0x1000_0300, 0, 0).unwrap();
            let got = uc.reg_read(RegisterARM::R0).unwrap();
            *uc.get_data_mut() = got as u32;
            uc.reg_write(RegisterARM::R0, r0).unwrap();
            uc.reg_write(RegisterARM::LR, lr).unwrap();
            uc.reg_write(RegisterARM::PC, lr).unwrap();
        })
        .unwrap();
        uc.reg_write(RegisterARM::R0, 1.5f32.to_bits() as u64).unwrap();
        uc.emu_start(0x1000_0001, 0x1000_000e, 0, 0).unwrap();
        assert_eq!(uc.reg_read(RegisterARM::R0).unwrap() as u32, 3.0f32.to_bits());
        assert_eq!(*uc.get_data(), 42);
    }
}
