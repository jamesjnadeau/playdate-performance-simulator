//! Basic-block costing: decode a block with Capstone once, sum the calibrated
//! cost of each instruction class, and cache it.

use capstone::arch::arm::{ArchExtraMode, ArchMode};
use capstone::prelude::*;
use pdsim_core::Calibration;

/// What a block does when it ends, for the shadow call stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Fallthrough,
    Call,
    Return,
}

/// Instruction classes, in the order of [`BlockInfo::classes`].
pub const CLASSES: [&str; 12] = [
    "alu",
    "mul",
    "div",
    "load",
    "store",
    "load_multiple",
    "branch",
    "call",
    "fp",
    "fp_div",
    "fp_sqrt",
    "fp_load",
];

/// Cost and shape of one basic block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockInfo {
    pub cycles: f64,
    pub instructions: u32,
    pub exit: Exit,
    /// Instructions per class (see [`CLASSES`]).
    pub classes: [u16; 12],
    /// Registers moved by load/store-multiple instructions.
    pub registers: u16,
}

/// Instruction class of a mnemonic (Capstone syntax, condition and width
/// suffixes included).
pub fn classify(mnemonic: &str, op_str: &str) -> (&'static str, u32) {
    let m = mnemonic.trim_end_matches(".w").trim_end_matches(".n");
    let regs = || {
        op_str
            .split_once('{')
            .and_then(|(_, r)| r.split_once('}'))
            .map_or(1, |(r, _)| {
                r.split(',').filter(|s| !s.trim().is_empty()).count() as u32
            })
    };
    if m.starts_with('v') {
        let class = if m.starts_with("vdiv") {
            "fp_div"
        } else if m.starts_with("vsqrt") {
            "fp_sqrt"
        } else if m.starts_with("vldr") || m.starts_with("vstr") {
            "fp_load"
        } else if m.starts_with("vpush")
            || m.starts_with("vpop")
            || m.starts_with("vldm")
            || m.starts_with("vstm")
        {
            return ("fp_load", regs());
        } else {
            "fp"
        };
        return (class, 0);
    }
    if m == "push" || m == "pop" || m.starts_with("ldm") || m.starts_with("stm") {
        return ("load_multiple", regs());
    }
    if m == "bl" || m == "blx" {
        return ("call", 0);
    }
    if m == "b"
        || m.starts_with("bx")
        || m.starts_with("cbz")
        || m.starts_with("cbnz")
        || m.starts_with("tbb")
        || m.starts_with("tbh")
        || (m.starts_with('b')
            && m.len() <= 3
            && !m.starts_with("bic")
            && !m.starts_with("bfc")
            && !m.starts_with("bfi"))
    {
        return ("branch", 0);
    }
    if m.starts_with("ldr") || m.starts_with("ldrex") {
        return ("load", 0);
    }
    if m.starts_with("str") {
        return ("store", 0);
    }
    if m.starts_with("sdiv") || m.starts_with("udiv") {
        return ("div", 0);
    }
    if m.starts_with("mul")
        || m.starts_with("mla")
        || m.starts_with("mls")
        || m.starts_with("smul")
        || m.starts_with("umul")
        || m.starts_with("smla")
        || m.starts_with("umla")
    {
        return ("mul", 0);
    }
    ("alu", 0)
}

fn is_return(mnemonic: &str, op_str: &str) -> bool {
    let m = mnemonic.trim_end_matches(".w");
    (m.starts_with("bx") && op_str.trim() == "lr")
        || ((m == "pop" || m.starts_with("ldm")) && op_str.contains("pc"))
        || (m.starts_with("ldr") && op_str.trim_start().starts_with("pc"))
}

/// Decodes and costs Thumb-2 code.
pub struct BlockCoster {
    cs: Capstone,
    costs: Vec<(&'static str, f64)>,
    per_register: f64,
    default: f64,
}

impl BlockCoster {
    pub fn new(cal: &Calibration) -> BlockCoster {
        let cs = Capstone::new()
            .arm()
            .mode(ArchMode::Thumb)
            .extra_mode([ArchExtraMode::MClass].iter().copied())
            .build()
            .expect("capstone");
        let classes = [
            "alu",
            "mul",
            "div",
            "load",
            "store",
            "load_multiple",
            "branch",
            "call",
            "fp",
            "fp_div",
            "fp_sqrt",
            "fp_load",
        ];
        BlockCoster {
            cs,
            costs: classes.iter().map(|c| (*c, cal.arm_class(c))).collect(),
            per_register: cal.arm.per_register,
            default: cal.arm.default_class,
        }
    }

    fn class_cost(&self, class: &str) -> f64 {
        self.costs
            .iter()
            .find(|(c, _)| *c == class)
            .map_or(self.default, |(_, v)| *v)
    }

    /// Costs the block of `size` bytes in `code` starting at `addr`.
    pub fn cost(&self, code: &[u8], addr: u64) -> BlockInfo {
        let Ok(insns) = self.cs.disasm_all(code, addr) else {
            return BlockInfo {
                cycles: self.default,
                instructions: 1,
                exit: Exit::Fallthrough,
                classes: [0; 12],
                registers: 0,
            };
        };
        let mut cycles = 0.0;
        let mut exit = Exit::Fallthrough;
        let mut n = 0;
        let mut classes = [0u16; 12];
        let mut registers = 0u16;
        for i in insns.iter() {
            let m = i.mnemonic().unwrap_or("");
            let o = i.op_str().unwrap_or("");
            let (class, regs) = classify(m, o);
            cycles += self.class_cost(class) + regs as f64 * self.per_register;
            if let Some(k) = CLASSES.iter().position(|c| *c == class) {
                classes[k] += 1;
            }
            registers += regs as u16;
            exit = if class == "call" {
                Exit::Call
            } else if is_return(m, o) {
                Exit::Return
            } else {
                Exit::Fallthrough
            };
            n += 1;
        }
        BlockInfo {
            cycles,
            instructions: n,
            exit,
            classes,
            registers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coster() -> (BlockCoster, Calibration) {
        let cal = Calibration::default_estimates();
        (BlockCoster::new(&cal), cal)
    }

    #[test]
    fn hand_assembled_alu_and_memory() {
        let (c, cal) = coster();
        // adds r0, #1 ; muls r0, r1, r0 ; ldr r0, [r1] ; str r0, [r1] ; bx lr
        let code = [0x01, 0x30, 0x48, 0x43, 0x08, 0x68, 0x08, 0x60, 0x70, 0x47];
        let b = c.cost(&code, 0x6000_0000);
        assert_eq!(b.instructions, 5);
        let want = cal.arm_class("alu")
            + cal.arm_class("mul")
            + cal.arm_class("load")
            + cal.arm_class("store")
            + cal.arm_class("branch");
        assert!((b.cycles - want).abs() < 1e-9, "{} vs {want}", b.cycles);
        assert_eq!(b.exit, Exit::Return);
    }

    #[test]
    fn push_pop_scale_with_registers() {
        let (c, cal) = coster();
        // push {r4, r5, lr} ; pop {r4, r5, pc}
        let b = c.cost(&[0x30, 0xb5, 0x30, 0xbd], 0);
        let want = 2.0 * (cal.arm_class("load_multiple") + 3.0 * cal.arm.per_register);
        assert!((b.cycles - want).abs() < 1e-9);
        assert_eq!(b.exit, Exit::Return);
    }

    #[test]
    fn divide_float_and_call() {
        let (c, cal) = coster();
        // sdiv r0, r0, r1 ; vadd.f32 s0, s0, s1 ; vdiv.f32 s0, s0, s1 ; bl +0
        let code = [
            0x90, 0xfb, 0xf1, 0xf0, 0x30, 0xee, 0x20, 0x0a, 0x80, 0xee, 0x20, 0x0a, 0x00, 0xf0, 0x00, 0xf8,
        ];
        let b = c.cost(&code, 0x1000);
        assert_eq!(b.instructions, 4);
        let want =
            cal.arm_class("div") + cal.arm_class("fp") + cal.arm_class("fp_div") + cal.arm_class("call");
        assert!((b.cycles - want).abs() < 1e-9, "{} vs {want}", b.cycles);
        assert_eq!(b.exit, Exit::Call);
    }

    #[test]
    fn classifies_mnemonics() {
        assert_eq!(classify("bne", "#0x10").0, "branch");
        assert_eq!(classify("bic", "r0, r1").0, "alu");
        assert_eq!(classify("ldr.w", "r0, [r1, #4]").0, "load");
        assert_eq!(classify("udiv", "r0, r1, r2").0, "div");
        assert_eq!(classify("vldr", "s0, [r0]").0, "fp_load");
        assert_eq!(classify("cbz", "r0, #4").0, "branch");
        assert_eq!(classify("smull", "r0, r1, r2, r3").0, "mul");
    }
}
