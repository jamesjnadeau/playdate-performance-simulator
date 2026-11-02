//! Access to the patched VM's counters, and turning them into cycles.

use pdsim_core::Calibration;
use std::ffi::{c_char, c_int, CStr};

/// Raw Lua state pointer type.
pub type LuaState = mlua::ffi::lua_State;

extern "C" {
    fn pdsim_num_opcodes() -> c_int;
    fn pdsim_opname(op: c_int) -> *const c_char;
    fn pdsim_counters(l: *mut LuaState, ops: *mut u64, mem: *mut u64);
    fn pdsim_native_instructions(l: *mut LuaState) -> u64;
}

/// Instructions executed by API-in-Lua chunks (named `=[pdsim]...`), which
/// are not charged as game code.
///
/// # Safety
/// `l` must be a live Lua state created by this crate's Lua library.
pub unsafe fn native_instructions(l: *mut LuaState) -> u64 {
    pdsim_native_instructions(l)
}

/// Chunk-name prefix for API-in-Lua code.
pub const NATIVE_CHUNK_PREFIX: &str = "=[pdsim]";

/// Maximum opcode count the patched VM tracks.
pub const MAX_OPS: usize = 128;

/// Number of opcodes in this Lua.
pub fn num_opcodes() -> usize {
    unsafe { pdsim_num_opcodes() as usize }
}

/// Opcode names, in opcode order (`MOVE`, `LOADI`, ...).
pub fn opcode_names() -> Vec<&'static str> {
    (0..num_opcodes())
        .map(|i| {
            unsafe { CStr::from_ptr(pdsim_opname(i as c_int)) }
                .to_str()
                .unwrap_or("?")
        })
        .collect()
}

/// A snapshot of the VM counters.
#[derive(Debug, Clone, PartialEq)]
pub struct Counters {
    pub ops: Vec<u64>,
    pub alloced: u64,
    pub freed: u64,
    pub gc_work: u64,
    /// C functions (standard library and others) called by game code.
    pub c_calls: u64,
}

impl Counters {
    /// Reads the counters of the state `l` belongs to.
    ///
    /// # Safety
    /// `l` must be a live Lua state created by this crate's Lua library.
    pub unsafe fn read(l: *mut LuaState) -> Counters {
        let n = num_opcodes();
        let mut ops = vec![0u64; n];
        let mut mem = [0u64; 4];
        pdsim_counters(l, ops.as_mut_ptr(), mem.as_mut_ptr());
        Counters {
            ops,
            alloced: mem[0],
            freed: mem[1],
            gc_work: mem[2],
            c_calls: mem[3],
        }
    }

    /// Counts since `earlier`.
    pub fn since(&self, earlier: &Counters) -> Counters {
        Counters {
            ops: self.ops.iter().zip(&earlier.ops).map(|(a, b)| a - b).collect(),
            alloced: self.alloced - earlier.alloced,
            freed: self.freed - earlier.freed,
            gc_work: self.gc_work - earlier.gc_work,
            c_calls: self.c_calls - earlier.c_calls,
        }
    }

    /// Total instructions.
    pub fn instructions(&self) -> u64 {
        self.ops.iter().sum()
    }
}

/// Per-opcode costs resolved from a calibration, indexed by opcode.
#[derive(Debug, Clone)]
pub struct CostTable {
    pub op: Vec<f64>,
    pub gc_alloc: f64,
    pub gc_free: f64,
    pub gc_work: f64,
    pub c_call: f64,
}

impl CostTable {
    pub fn new(cal: &Calibration) -> Self {
        CostTable {
            op: opcode_names().iter().map(|n| cal.lua_opcode(n)).collect(),
            gc_alloc: cal.lua.gc_per_alloc_byte,
            gc_free: cal.lua.gc_per_free_byte,
            gc_work: cal.lua.gc_per_work_unit,
            c_call: cal.lua.c_function_call,
        }
    }

    /// Cycles for executing the opcodes in `c` and the C functions called.
    pub fn vm_cycles(&self, c: &Counters) -> f64 {
        c.ops
            .iter()
            .zip(&self.op)
            .map(|(n, k)| *n as f64 * k)
            .sum::<f64>()
            + c.c_calls as f64 * self.c_call
    }

    /// Cycles for allocation and collection in `c`.
    pub fn gc_cycles(&self, c: &Counters) -> f64 {
        c.alloced as f64 * self.gc_alloc + c.freed as f64 * self.gc_free + c.gc_work as f64 * self.gc_work
    }
}

/// The raw state pointer of an mlua instance.
pub fn raw_state(lua: &mlua::Lua) -> *mut LuaState {
    let mut p: *mut LuaState = std::ptr::null_mut();
    unsafe {
        lua.exec_raw::<()>((), |l| p = l).expect("exec_raw");
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcode_names_match_lua_54() {
        let n = opcode_names();
        assert_eq!(n[0], "MOVE");
        assert!(n.contains(&"FORLOOP"));
        assert!(n.len() <= MAX_OPS);
        // Every opcode has a cost in the built-in calibration.
        let cal = Calibration::default_estimates();
        for name in &n {
            assert!(cal.lua.opcodes.contains_key(*name), "no cost for {name}");
        }
    }

    #[test]
    fn fixed_loop_gives_expected_histogram() {
        let lua = mlua::Lua::new();
        let l = raw_state(&lua);
        let names = opcode_names();
        let idx = |n: &str| names.iter().position(|x| *x == n).unwrap();
        let f = lua
            .load("local x = 0 for i = 1, 10 do x = x + i end return x")
            .into_function()
            .unwrap();
        let before = unsafe { Counters::read(l) };
        let r: i64 = f.call(()).unwrap();
        let d = unsafe { Counters::read(l) }.since(&before);
        assert_eq!(r, 55);
        assert_eq!(d.ops[idx("ADD")], 10);
        assert_eq!(d.ops[idx("FORLOOP")], 10);
        assert_eq!(d.ops[idx("FORPREP")], 1);
        assert_eq!(d.ops[idx("MMBIN")], 0, "integer ADD skips its MMBIN");
        // Doubling the loop doubles the loop opcodes.
        let f2 = lua
            .load("local x = 0 for i = 1, 20 do x = x + i end return x")
            .into_function()
            .unwrap();
        let before = unsafe { Counters::read(l) };
        let _: i64 = f2.call(()).unwrap();
        let d2 = unsafe { Counters::read(l) }.since(&before);
        assert_eq!(d2.ops[idx("ADD")], 20);
        let costs = CostTable::new(&Calibration::default_estimates());
        assert!(costs.vm_cycles(&d2) > costs.vm_cycles(&d));
    }

    #[test]
    fn native_chunks_are_not_counted_as_game_code() {
        let lua = mlua::Lua::new();
        let l = raw_state(&lua);
        let f: mlua::Function = lua
            .load("return function(n) local x = 0 for i = 1, n do x = x + i end return x end")
            .set_name("=[pdsim]test")
            .eval()
            .unwrap();
        let before = unsafe { Counters::read(l) };
        let nb = unsafe { native_instructions(l) };
        let _: i64 = f.call(100).unwrap();
        let d = unsafe { Counters::read(l) }.since(&before);
        assert_eq!(d.instructions(), 0);
        assert!(unsafe { native_instructions(l) } - nb >= 200);
        // A game function calling it is still counted.
        lua.globals().set("nf", f).unwrap();
        let before = unsafe { Counters::read(l) };
        lua.load("nf(100)").exec().unwrap();
        let d = unsafe { Counters::read(l) }.since(&before);
        assert!(
            d.instructions() > 0 && d.instructions() < 10,
            "{}",
            d.instructions()
        );
    }

    #[test]
    fn c_function_calls_are_counted_for_game_code_only() {
        let lua = mlua::Lua::new();
        let l = raw_state(&lua);
        let before = unsafe { Counters::read(l) };
        lua.load("local s = 0 for i = 1, 10 do s = s + math.abs(-i) end")
            .exec()
            .unwrap();
        let d = unsafe { Counters::read(l) }.since(&before);
        assert_eq!(d.c_calls, 10);
        let f: mlua::Function = lua
            .load("return function() for i = 1, 10 do math.abs(-i) end end")
            .set_name("=[pdsim]t")
            .eval()
            .unwrap();
        let before = unsafe { Counters::read(l) };
        f.call::<()>(()).unwrap();
        assert_eq!(unsafe { Counters::read(l) }.since(&before).c_calls, 0);
    }

    #[test]
    fn allocation_is_counted() {
        let lua = mlua::Lua::new();
        let l = raw_state(&lua);
        let before = unsafe { Counters::read(l) };
        lua.load("local t = {} for i = 1, 1000 do t[i] = {i} end")
            .exec()
            .unwrap();
        let d = unsafe { Counters::read(l) }.since(&before);
        assert!(d.alloced > 1000 * 16, "alloced {}", d.alloced);
    }
}
