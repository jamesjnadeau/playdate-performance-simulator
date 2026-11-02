//! State shared by all `playdate.*` bindings.

use crate::vm::{CostTable, Counters, LuaState};
use mlua::Lua;
use pdsim_core::{Category, Work};
use pdsim_device::{Device, SharedDevice};
use std::cell::{Cell, RefCell, RefMut};
use std::rc::Rc;

/// Shared binding context.
pub struct Ctx {
    pub dev: SharedDevice,
    pub costs: CostTable,
    raw: *mut LuaState,
    last: RefCell<Counters>,
    /// Whether to attribute cycles to call stacks.
    pub profiling: Cell<bool>,
    /// Bitmaps whose Lua objects were collected (freed at frame end, since
    /// finalizers can run while the device is borrowed).
    pub dead_bitmaps: RefCell<Vec<u32>>,
    pub dead_sprites: RefCell<Vec<u32>>,
    pub native_call_overhead: f64,
}

pub type CtxRef = Rc<Ctx>;

impl Ctx {
    pub fn new(dev: SharedDevice, lua: &Lua) -> CtxRef {
        let raw = crate::vm::raw_state(lua);
        let (costs, overhead) = {
            let d = dev.borrow();
            (CostTable::new(&d.cal), d.cal.lua.native_call_overhead)
        };
        let last = unsafe { Counters::read(raw) };
        Rc::new(Ctx {
            dev,
            costs,
            raw,
            last: RefCell::new(last),
            profiling: Cell::new(true),
            dead_bitmaps: RefCell::new(vec![]),
            dead_sprites: RefCell::new(vec![]),
            native_call_overhead: overhead,
        })
    }

    /// Mutable access to the device.
    pub fn dev(&self) -> RefMut<'_, Device> {
        self.dev.borrow_mut()
    }

    /// Charges VM and GC work done since the last sync to the ledger and,
    /// when profiling, attributes it to the current Lua stack. Returns the
    /// cycles charged.
    pub fn sync(&self, lua: &Lua) -> f64 {
        let now = unsafe { Counters::read(self.raw) };
        let delta = now.since(&self.last.borrow());
        *self.last.borrow_mut() = now;
        let vm = self.costs.vm_cycles(&delta);
        let gc = self.costs.gc_cycles(&delta);
        {
            let mut d = self.dev();
            d.charge(Category::Lua, vm);
            d.charge(Category::Gc, gc);
        }
        if self.profiling.get() && vm + gc > 0.0 {
            let mut stack = lua_stack(lua);
            if stack.is_empty() {
                stack.push("(toplevel)".into());
            }
            let mut d = self.dev();
            d.profile.add_stack(&stack, vm.round() as u64);
            if gc > 0.0 {
                stack.push("[gc]".into());
                d.profile.add_stack(&stack, gc.round() as u64);
            }
        }
        vm + gc
    }

    /// Charges a native API call made from Lua: the calibrated cost of
    /// `name` for `work` in category `cat`, plus the Lua-to-native call
    /// overhead. Attributes it to the calling Lua function.
    pub fn api(&self, lua: &Lua, cat: Category, name: &str, work: Work) {
        let c = {
            let mut d = self.dev();
            d.charge(Category::Lua, self.native_call_overhead);
            d.charge_api(cat, name, work) + self.native_call_overhead
        };
        if self.profiling.get() {
            let mut stack = lua_stack(lua);
            stack.push(format!("[{name}]"));
            self.dev().profile.add_stack(&stack, c.round() as u64);
        }
    }
}

/// The current Lua call stack, outermost first, as `name source:line`.
pub fn lua_stack(lua: &Lua) -> Vec<String> {
    let mut v = vec![];
    for level in 0..48 {
        let Some(name) = lua.inspect_stack(level, |d| {
            let src = d.source();
            let names = d.names();
            let what = src.what;
            let short = src.short_src.as_deref().unwrap_or("?").to_string();
            let fname = names.name.as_deref().map(|s| s.to_string());
            if what == "C" {
                format!("[C] {}", fname.unwrap_or_else(|| "?".into()))
            } else if what == "main" {
                format!("{short} (main chunk)")
            } else {
                format!(
                    "{} {}:{}",
                    fname.unwrap_or_else(|| "?".into()),
                    short,
                    src.line_defined.unwrap_or(0)
                )
            }
        }) else {
            break;
        };
        v.push(name);
    }
    // Drop native frames at the top (the binding itself) and the simulator's
    // API-in-Lua frames anywhere: their cost belongs to the game function
    // that called the API.
    while v.first().is_some_and(|f| f.starts_with("[C]")) {
        v.remove(0);
    }
    v.retain(|f| !f.contains(" [pdsim]"));
    v.reverse();
    v
}
