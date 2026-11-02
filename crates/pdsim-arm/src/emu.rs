//! The emulated Cortex-M7 running a C game.

use crate::abi::{Arg, Machine};
use crate::cost::{BlockCoster, BlockInfo, Exit};
use crate::elf::{self, Image};
use crate::heap::Heap;
use crate::layout::{Kind, Layout, Member, Signature};
use pdsim_core::{Category, FrameStats};
use pdsim_device::{Game, GameError, InputScript, SharedDevice};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use unicorn_engine::unicorn_const::{Arch, Mode, Prot};
use unicorn_engine::{ArmCpuModel, RegisterARM, Unicorn};

/// Where the game's code is loaded (as on the device).
pub const CODE_BASE: u32 = 0x6000_0000;
/// The game heap (`pd->system->realloc`).
pub const HEAP_BASE: u32 = 0x6100_0000;
pub const HEAP_SIZE: u32 = 8 * 1024 * 1024;
/// The stack (grows down from `STACK_TOP`).
pub const STACK_BASE: u32 = 0x6200_0000;
pub const STACK_SIZE: u32 = 0x10000;
pub const STACK_TOP: u32 = STACK_BASE + STACK_SIZE;
/// `PlaydateAPI` structs and simulator-owned buffers.
pub const API_BASE: u32 = 0x6300_0000;
pub const API_SIZE: u32 = 0x40000;
/// Framebuffer mirror for `getFrame`, inside the API region.
pub const FRAME_ADDR: u32 = API_BASE + 0x20000;
pub const DISPLAY_ADDR: u32 = API_BASE + 0x24000;
/// Scratch for strings returned to the game.
pub const SCRATCH_ADDR: u32 = API_BASE + 0x28000;
/// One `bx lr` per API function; calls are intercepted by a hook.
pub const TRAMP_BASE: u32 = 0x6400_0000;
pub const TRAMP_SIZE: u32 = 0x10000;
/// Where callbacks into game code return to (stops the emulator).
pub const RETURN_ADDR: u32 = TRAMP_BASE + TRAMP_SIZE - 0x10;

/// Opaque handle bases (never dereferenced by games).
pub const BITMAP_BASE: u32 = 0x7100_0000;
pub const FONT_BASE: u32 = 0x7200_0000;
pub const SPRITE_BASE: u32 = 0x7300_0000;
pub const FILE_BASE: u32 = 0x7400_0000;
pub const MENU_BASE: u32 = 0x7500_0000;
pub const TABLE_BASE: u32 = 0x7600_0000;
pub const SOUND_BASE: u32 = 0x7700_0000;

/// Callbacks a sprite has registered.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpriteFns {
    pub update: u32,
    pub draw: u32,
    pub collision: u32,
    pub userdata: u32,
}

/// Emulator-side state, reachable from hooks.
pub struct ArmState {
    pub dev: SharedDevice,
    pub image: Image,
    pub funcs: Vec<(String, Signature)>,
    pub api_root: u32,
    coster: BlockCoster,
    blocks: HashMap<u32, (BlockInfo, u32)>,
    /// C cycles not yet charged to the ledger.
    pending: f64,
    // Shadow call stack (symbol indexes; u32::MAX = unknown).
    stack: Vec<u32>,
    /// Calls beyond MAX_STACK_DEPTH, which are not tracked.
    stack_overflow: u32,
    stack_id: usize,
    stack_ids: HashMap<Vec<u32>, usize>,
    stack_keys: Vec<Vec<u32>>,
    stack_cycles: Vec<f64>,
    pub last_exit: Exit,
    frame_cycles: f64,
    watchdog: f64,
    pub heap: Heap,
    pub update_cb: Option<(u32, u32)>,
    pub sprite_fns: HashMap<u32, SpriteFns>,
    pub error: Option<String>,
    pub frame_mirror: bool,
    pub unimplemented: HashSet<usize>,
    pub files: HashMap<u32, crate::api::OpenFile>,
    pub next_file: u32,
    pub last_file_error: Option<String>,
    pub tables: HashMap<u32, Vec<u32>>,
    pub next_table: u32,
    pub scratch_next: u32,
    pub profile: bool,
    pub button_cb: Option<(u32, u32)>,
    /// Instructions per class not yet reported as features.
    class_counts: [f64; 12],
    registers: f64,
}

/// Deepest shadow call stack kept; each distinct stack is interned in full.
const MAX_STACK_DEPTH: usize = 128;

impl ArmState {
    fn intern(&mut self) {
        if let Some(id) = self.stack_ids.get(&self.stack) {
            self.stack_id = *id;
        } else {
            let id = self.stack_keys.len();
            self.stack_ids.insert(self.stack.clone(), id);
            self.stack_keys.push(self.stack.clone());
            self.stack_cycles.push(0.0);
            self.stack_id = id;
        }
    }

    /// Current shadow stack as names (outermost first).
    pub fn stack_names(&self) -> Vec<String> {
        self.stack.iter().map(|s| self.sym_name(*s)).collect()
    }

    fn sym_name(&self, s: u32) -> String {
        self.image
            .symbols
            .get(s as usize)
            .map_or_else(|| "(unknown)".into(), |x| x.name.clone())
    }

    /// Charges pending C cycles to the ledger.
    pub fn sync(&mut self) {
        let mut d = self.dev.borrow_mut();
        if self.pending > 0.0 {
            d.charge(Category::C, self.pending);
            self.pending = 0.0;
        }
        if let Some(f) = &mut d.features {
            for (i, c) in crate::cost::CLASSES.iter().enumerate() {
                f.add(&format!("arm.{c}"), self.class_counts[i]);
            }
            f.add("arm.registers", self.registers);
        }
        self.class_counts = [0.0; 12];
        self.registers = 0.0;
    }

    /// Moves per-stack cycles into the device's frame profile.
    fn flush_profile(&mut self) {
        let mut d = self.dev.borrow_mut();
        for (i, c) in self.stack_cycles.iter_mut().enumerate() {
            if *c > 0.0 {
                let names: Vec<String> = self.stack_keys[i]
                    .iter()
                    .map(|s| {
                        self.image
                            .symbols
                            .get(*s as usize)
                            .map_or_else(|| "(unknown)".into(), |x| x.name.clone())
                    })
                    .collect();
                if !names.is_empty() {
                    d.profile.add_stack(&names, c.round() as u64);
                }
                *c = 0.0;
            }
        }
    }

    /// Charges an API call and attributes it to the calling C function.
    pub fn charge_api(&mut self, cat: Category, name: &str, work: pdsim_core::Work) {
        self.sync();
        let c = self.dev.borrow_mut().charge_api(cat, name, work);
        if self.profile && c > 0.0 {
            let mut st = self.stack_names();
            st.push(format!("[{name}]"));
            self.dev.borrow_mut().profile.add_stack(&st, c.round() as u64);
        }
    }

    /// Bytes for a string handed to the game, from a ring of scratch space.
    pub fn scratch(&mut self, len: u32) -> u32 {
        let len = (len + 8).div_ceil(8) * 8;
        if self.scratch_next + len > API_BASE + API_SIZE {
            self.scratch_next = SCRATCH_ADDR;
        }
        let a = self.scratch_next;
        self.scratch_next += len;
        a
    }
}

/// Register access for the ABI decoder.
pub struct Regs<'a, 'b>(pub &'a Unicorn<'b, ArmState>);

impl Machine for Regs<'_, '_> {
    fn core(&self, n: usize) -> u32 {
        let r = [RegisterARM::R0, RegisterARM::R1, RegisterARM::R2, RegisterARM::R3][n];
        self.0.reg_read(r).unwrap_or(0) as u32
    }
    fn sreg(&self, n: usize) -> f32 {
        f32::from_bits(self.0.reg_read(RegisterARM::S0 as i32 + n as i32).unwrap_or(0) as u32)
    }
    fn sp(&self) -> u32 {
        self.0.reg_read(RegisterARM::SP).unwrap_or(0) as u32
    }
    fn read_u32(&self, addr: u32) -> u32 {
        let mut b = [0u8; 4];
        let _ = self.0.mem_read(addr as u64, &mut b);
        u32::from_le_bytes(b)
    }
}

/// Memory helpers on the emulator.
pub trait Mem {
    fn r32(&self, a: u32) -> u32;
    fn w32(&mut self, a: u32, v: u32);
    fn wf32(&mut self, a: u32, v: f32) {
        self.w32(a, v.to_bits())
    }
    fn rf32(&self, a: u32) -> f32 {
        f32::from_bits(self.r32(a))
    }
    fn cstr(&self, a: u32) -> String;
    fn bytes(&self, a: u32, n: usize) -> Vec<u8>;
    fn write(&mut self, a: u32, b: &[u8]);
}

impl Mem for Unicorn<'_, ArmState> {
    fn r32(&self, a: u32) -> u32 {
        let mut b = [0u8; 4];
        let _ = self.mem_read(a as u64, &mut b);
        u32::from_le_bytes(b)
    }
    fn w32(&mut self, a: u32, v: u32) {
        if a != 0 {
            let _ = self.mem_write(a as u64, &v.to_le_bytes());
        }
    }
    fn cstr(&self, a: u32) -> String {
        if a == 0 {
            return String::new();
        }
        let mut out = vec![];
        let mut p = a as u64;
        let mut chunk = [0u8; 64];
        loop {
            if self.mem_read(p, &mut chunk).is_err() {
                // Read byte by byte near the end of a region.
                let mut b = [0u8; 1];
                if self.mem_read(p, &mut b).is_err() || b[0] == 0 {
                    break;
                }
                out.push(b[0]);
                p += 1;
                continue;
            }
            if let Some(z) = chunk.iter().position(|c| *c == 0) {
                out.extend_from_slice(&chunk[..z]);
                break;
            }
            out.extend_from_slice(&chunk);
            p += 64;
            if out.len() > 1 << 20 {
                break;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }
    fn bytes(&self, a: u32, n: usize) -> Vec<u8> {
        self.mem_read_as_vec(a as u64, n).unwrap_or_default()
    }
    fn write(&mut self, a: u32, b: &[u8]) {
        let _ = self.mem_write(a as u64, b);
    }
}

/// Sets a return value.
pub fn set_ret(uc: &mut Unicorn<'_, ArmState>, kind: Option<Kind>, v: Ret) {
    match (kind, v) {
        (_, Ret::None) | (None, _) => {}
        (Some(Kind::Float), Ret::F(f)) => {
            let _ = uc.reg_write(RegisterARM::S0, f.to_bits() as u64);
        }
        (Some(Kind::PdRect), Ret::Rect(r)) => {
            for (i, f) in r.iter().enumerate() {
                let _ = uc.reg_write(RegisterARM::S0 as i32 + i as i32, f.to_bits() as u64);
            }
        }
        (Some(Kind::Int64), Ret::I(i)) => {
            let _ = uc.reg_write(RegisterARM::R0, i as u64);
            let _ = uc.reg_write(RegisterARM::R1, 0);
        }
        (_, Ret::I(i)) => {
            let _ = uc.reg_write(RegisterARM::R0, i as u64);
        }
        (_, Ret::F(f)) => {
            let _ = uc.reg_write(RegisterARM::R0, f as i32 as u32 as u64);
        }
        (_, Ret::Rect(_)) => {}
    }
}

/// A return value from an API implementation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ret {
    None,
    I(u32),
    F(f32),
    Rect([f32; 4]),
}

const CALLEE_SAVED: [RegisterARM; 14] = [
    RegisterARM::R0,
    RegisterARM::R1,
    RegisterARM::R2,
    RegisterARM::R3,
    RegisterARM::R4,
    RegisterARM::R5,
    RegisterARM::R6,
    RegisterARM::R7,
    RegisterARM::R8,
    RegisterARM::R9,
    RegisterARM::R10,
    RegisterARM::R11,
    RegisterARM::R12,
    RegisterARM::LR,
];

/// Calls game function `f` with integer arguments; returns r0. Usable from
/// inside an API hook (nested emulation); registers are preserved.
pub fn call(uc: &mut Unicorn<'_, ArmState>, f: u32, args: &[u32]) -> Result<u32, String> {
    call_full(uc, f, args, &[])
}

/// Like [`call`], also passing single-precision arguments in s0, s1, ...
pub fn call_full(uc: &mut Unicorn<'_, ArmState>, f: u32, args: &[u32], fargs: &[f32]) -> Result<u32, String> {
    if f == 0 {
        return Ok(0);
    }
    let saved: Vec<u64> = CALLEE_SAVED
        .iter()
        .map(|r| uc.reg_read(*r).unwrap_or(0))
        .collect();
    let sp = uc.reg_read(RegisterARM::SP).unwrap_or(STACK_TOP as u64);
    let pc = uc.reg_read(RegisterARM::PC).unwrap_or(0);
    let st_saved = {
        let st = uc.get_data_mut();
        let s = (st.stack.clone(), st.stack_id, st.last_exit);
        st.last_exit = Exit::Call;
        s
    };
    let regs = [RegisterARM::R0, RegisterARM::R1, RegisterARM::R2, RegisterARM::R3];
    for (i, a) in args.iter().take(4).enumerate() {
        let _ = uc.reg_write(regs[i], *a as u64);
    }
    // Further arguments go on the stack.
    let mut sp_new = (sp as u32) & !7;
    if args.len() > 4 {
        let extra = &args[4..];
        sp_new -= (extra.len() as u32 * 4).div_ceil(8) * 8;
        for (i, a) in extra.iter().enumerate() {
            uc.w32(sp_new + 4 * i as u32, *a);
        }
    }
    for (i, v) in fargs.iter().take(16).enumerate() {
        let _ = uc.reg_write(RegisterARM::S0 as i32 + i as i32, v.to_bits() as u64);
    }
    let _ = uc.reg_write(RegisterARM::SP, sp_new as u64);
    let _ = uc.reg_write(RegisterARM::LR, (RETURN_ADDR | 1) as u64);
    let r = uc.emu_start((f | 1) as u64, RETURN_ADDR as u64, 0, 0);
    // Describe a fault now: the registers are restored below.
    let crash = r.err().map(|e| crash_message(uc, e));
    let ret = uc.reg_read(RegisterARM::R0).unwrap_or(0) as u32;
    for (r, v) in CALLEE_SAVED.iter().zip(saved) {
        let _ = uc.reg_write(*r, v);
    }
    let _ = uc.reg_write(RegisterARM::SP, sp);
    let _ = uc.reg_write(RegisterARM::PC, pc);
    {
        let st = uc.get_data_mut();
        st.stack = st_saved.0;
        st.stack_id = st_saved.1;
        st.last_exit = st_saved.2;
    }
    if let Some(e) = uc.get_data().error.clone() {
        return Err(e);
    }
    if let Some(msg) = crash {
        return Err(msg);
    }
    Ok(ret)
}

fn crash_message(uc: &Unicorn<'_, ArmState>, e: unicorn_engine::unicorn_const::uc_error) -> String {
    let pc = uc.reg_read(RegisterARM::PC).unwrap_or(0) as u32;
    let st = uc.get_data();
    let sym = st
        .image
        .symbol_at(pc)
        .map(|i| st.image.symbols[i].name.clone())
        .unwrap_or_else(|| "?".into());
    let lr = uc.reg_read(RegisterARM::LR).unwrap_or(0) as u32;
    let lr_sym = st
        .image
        .symbol_at(lr & !1)
        .map(|i| st.image.symbols[i].name.clone())
        .unwrap_or_else(|| "?".into());
    format!("C game crashed: {e:?} at pc {pc:#010x} ({sym}), lr {lr:#010x} ({lr_sym})")
}

/// A C game running in the emulator.
pub struct ArmGame {
    uc: Unicorn<'static, ArmState>,
    dev: SharedDevice,
    path: String,
}

fn gerr(e: impl std::fmt::Display) -> GameError {
    GameError(e.to_string())
}

fn uerr(what: &str) -> impl Fn(unicorn_engine::unicorn_const::uc_error) -> GameError + '_ {
    move |e| GameError(format!("{what}: {e:?}"))
}

/// Builds the API structs in memory: returns the bytes of the API region, the
/// address of the root `PlaydateAPI`, and the function table in trampoline
/// order.
fn build_api(layout: &Layout) -> (Vec<u8>, u32, Vec<(String, Signature)>) {
    let mut region = vec![0u8; 0x20000];
    let mut next = 0u32;
    let mut funcs = vec![];
    fn place(
        layout: &Layout,
        ty: &str,
        prefix: &str,
        region: &mut Vec<u8>,
        next: &mut u32,
        funcs: &mut Vec<(String, Signature)>,
        depth: usize,
    ) -> u32 {
        let members = layout.structs.get(ty).cloned().unwrap_or_default();
        let at = *next;
        *next += (members.len() as u32 * 4).max(4);
        for (i, m) in members.iter().enumerate() {
            let word = match m {
                Member::Function { name, sig } => {
                    let idx = funcs.len() as u32;
                    funcs.push((format!("{prefix}{name}"), sig.clone()));
                    (TRAMP_BASE + idx * 4) | 1
                }
                Member::Struct { name, ty } if depth < 6 => {
                    API_BASE
                        + place(
                            layout,
                            ty,
                            &format!("{prefix}{name}."),
                            region,
                            next,
                            funcs,
                            depth + 1,
                        )
                }
                Member::Struct { .. } => 0,
            };
            let o = (at + i as u32 * 4) as usize;
            region[o..o + 4].copy_from_slice(&word.to_le_bytes());
        }
        at
    }
    let root = place(layout, "PlaydateAPI", "", &mut region, &mut next, &mut funcs, 0);
    (region, API_BASE + root, funcs)
}

impl ArmGame {
    /// Loads `elf` (a Playdate device build) using the API layout from `sdk`.
    pub fn load(
        dev: SharedDevice,
        elf_path: &Path,
        sdk: Option<&Path>,
        profile: bool,
    ) -> Result<ArmGame, GameError> {
        let sdk = sdk.ok_or_else(|| {
            GameError("C games need the Playdate SDK (set PLAYDATE_SDK_PATH): the API layout is read from its headers".into())
        })?;
        let layout = Layout::from_sdk(sdk).map_err(|e| GameError(format!("reading SDK headers: {e}")))?;
        let data = std::fs::read(elf_path).map_err(|e| GameError(format!("{}: {e}", elf_path.display())))?;
        let image = elf::load(&data, CODE_BASE).map_err(gerr)?;
        let (api_bytes, api_root, funcs) = build_api(&layout);
        let coster = BlockCoster::new(&dev.borrow().cal);
        let state = ArmState {
            dev: dev.clone(),
            image,
            funcs,
            api_root,
            coster,
            blocks: HashMap::new(),
            pending: 0.0,
            stack: vec![],
            stack_overflow: 0,
            stack_id: 0,
            stack_ids: HashMap::new(),
            stack_keys: vec![],
            stack_cycles: vec![],
            last_exit: Exit::Call,
            frame_cycles: 0.0,
            watchdog: dev.borrow().cal.cpu_hz as f64 * 10.0,
            heap: Heap::new(HEAP_BASE, HEAP_SIZE),
            update_cb: None,
            sprite_fns: HashMap::new(),
            error: None,
            frame_mirror: false,
            unimplemented: HashSet::new(),
            files: HashMap::new(),
            next_file: 1,
            last_file_error: None,
            tables: HashMap::new(),
            next_table: 1,
            scratch_next: SCRATCH_ADDR,
            profile,
            button_cb: None,
            class_counts: [0.0; 12],
            registers: 0.0,
        };
        let mut uc =
            Unicorn::new_with_data(Arch::ARM, Mode::THUMB | Mode::MCLASS, state).map_err(uerr("unicorn"))?;
        uc.ctl_set_cpu_model(ArmCpuModel::CORTEX_M7.into())
            .map_err(uerr("cpu model"))?;
        let code_len = (uc.get_data().image.bytes.len() as u64).div_ceil(0x10000) * 0x10000;
        uc.mem_map(CODE_BASE as u64, code_len.max(0x10000), Prot::ALL)
            .map_err(uerr("map code"))?;
        let bytes = uc.get_data().image.bytes.clone();
        uc.mem_write(CODE_BASE as u64, &bytes)
            .map_err(uerr("write code"))?;
        uc.mem_map(HEAP_BASE as u64, HEAP_SIZE as u64, Prot::READ | Prot::WRITE)
            .map_err(uerr("map heap"))?;
        uc.mem_map(STACK_BASE as u64, STACK_SIZE as u64, Prot::READ | Prot::WRITE)
            .map_err(uerr("map stack"))?;
        uc.mem_map(API_BASE as u64, API_SIZE as u64, Prot::READ | Prot::WRITE)
            .map_err(uerr("map api"))?;
        uc.mem_write(API_BASE as u64, &api_bytes)
            .map_err(uerr("write api"))?;
        uc.mem_map(TRAMP_BASE as u64, TRAMP_SIZE as u64, Prot::READ | Prot::EXEC)
            .map_err(uerr("map trampolines"))?;
        let tramp: Vec<u8> = std::iter::repeat_n([0x70u8, 0x47], TRAMP_SIZE as usize / 2)
            .flatten()
            .collect();
        uc.mem_write(TRAMP_BASE as u64, &tramp)
            .map_err(uerr("write trampolines"))?;

        // Cost every basic block of game code.
        let text_end = uc.get_data().image.text_end;
        uc.add_block_hook(CODE_BASE as u64, text_end as u64, |uc, addr, size| {
            let a = addr as u32;
            if !uc.get_data().blocks.contains_key(&a) {
                let code = uc.mem_read_as_vec(addr, size as usize).unwrap_or_default();
                let st = uc.get_data_mut();
                let info = st.coster.cost(&code, addr);
                let sym = st.image.symbol_at(a).map_or(u32::MAX, |i| i as u32);
                st.blocks.insert(a, (info, sym));
            }
            let st = uc.get_data_mut();
            let (info, sym) = st.blocks[&a];
            match st.last_exit {
                Exit::Call if st.stack.len() >= MAX_STACK_DEPTH => st.stack_overflow += 1,
                Exit::Call => {
                    st.stack.push(sym);
                    st.intern();
                }
                Exit::Return if st.stack_overflow > 0 => st.stack_overflow -= 1,
                Exit::Return => {
                    st.stack.pop();
                    if st.stack.last() != Some(&sym) {
                        if let Some(t) = st.stack.last_mut() {
                            *t = sym;
                        } else {
                            st.stack.push(sym);
                        }
                    }
                    st.intern();
                }
                Exit::Fallthrough => {
                    if st.stack.last() != Some(&sym) {
                        if let Some(t) = st.stack.last_mut() {
                            *t = sym;
                        } else {
                            st.stack.push(sym);
                        }
                        st.intern();
                    }
                }
            }
            st.last_exit = info.exit;
            st.pending += info.cycles;
            for (k, n) in info.classes.iter().enumerate() {
                st.class_counts[k] += *n as f64;
            }
            st.registers += info.registers as f64;
            st.frame_cycles += info.cycles;
            let id = st.stack_id;
            if let Some(c) = st.stack_cycles.get_mut(id) {
                *c += info.cycles;
            }
            if st.frame_cycles > st.watchdog && st.error.is_none() {
                st.error =
                    Some("C game ran for over 10 s of device time in one frame (infinite loop?)".into());
                let _ = uc.emu_stop();
            }
        })
        .map_err(uerr("block hook"))?;
        // API calls.
        uc.add_code_hook(TRAMP_BASE as u64, (RETURN_ADDR - 1) as u64, |uc, addr, _| {
            let idx = ((addr as u32 - TRAMP_BASE) / 4) as usize;
            let lr = uc.reg_read(RegisterARM::LR).unwrap_or(0);
            uc.get_data_mut().last_exit = Exit::Fallthrough;
            crate::api::dispatch(uc, idx);
            if uc.get_data().error.is_some() {
                let _ = uc.emu_stop();
            }
            let _ = uc.reg_write(RegisterARM::PC, lr);
        })
        .map_err(uerr("code hook"))?;

        let mut g = ArmGame {
            uc,
            dev,
            path: elf_path.display().to_string(),
        };
        // kEventInit
        let entry = g.uc.get_data().image.entry;
        let root = g.uc.get_data().api_root;
        g.top_call(entry, &[root, 0, 0])?;
        g.uc.get_data_mut().sync();
        g.uc.get_data_mut().flush_profile();
        {
            let mut d = g.dev.borrow_mut();
            d.profile = Default::default();
            d.ledger.take_frame();
        }
        if g.uc.get_data().update_cb.is_none() {
            return Err(GameError(
                "the game did not call setUpdateCallback during kEventInit (Lua+C hybrid games are not supported)".into(),
            ));
        }
        Ok(g)
    }

    fn top_call(&mut self, f: u32, args: &[u32]) -> Result<u32, GameError> {
        let _ = self.uc.reg_write(RegisterARM::SP, STACK_TOP as u64);
        {
            let st = self.uc.get_data_mut();
            st.stack.clear();
            st.intern();
            st.frame_cycles = 0.0;
        }
        call(&mut self.uc, f, args).map_err(GameError)
    }

    /// The emulator (for tests and tools).
    pub fn emulator(&mut self) -> &mut Unicorn<'static, ArmState> {
        &mut self.uc
    }

    /// Calls a function in the game with integer arguments.
    pub fn call_function(&mut self, f: u32, args: &[u32]) -> Result<u32, GameError> {
        self.top_call(f, args)
    }

    fn frame_to_emu(&mut self) {
        let data = self.dev.borrow().gfx.frame().data.clone();
        let _ = self.uc.mem_write(FRAME_ADDR as u64, &data);
    }

    fn frame_from_emu(&mut self) {
        let n = pdsim_core::LCD_ROWSIZE * pdsim_core::LCD_HEIGHT;
        let data = self.uc.bytes(FRAME_ADDR, n);
        if data.len() != n {
            return;
        }
        let mut d = self.dev.borrow_mut();
        let rs = pdsim_core::LCD_ROWSIZE;
        let mut changed = vec![];
        {
            let f = d.gfx.frame_mut();
            for y in 0..pdsim_core::LCD_HEIGHT {
                if f.data[y * rs..(y + 1) * rs] != data[y * rs..(y + 1) * rs] {
                    f.data[y * rs..(y + 1) * rs].copy_from_slice(&data[y * rs..(y + 1) * rs]);
                    changed.push(y as i32);
                }
            }
        }
        for y in changed {
            d.gfx.mark_updated_rows(y, y);
        }
    }

    /// Syncs the frame mirror before an API call that reads or draws the
    /// frame (only when the game uses `getFrame`).
    pub fn before_gfx(uc: &mut Unicorn<'_, ArmState>) {
        if !uc.get_data().frame_mirror {
            return;
        }
        let n = pdsim_core::LCD_ROWSIZE * pdsim_core::LCD_HEIGHT;
        let data = uc.bytes(FRAME_ADDR, n);
        let st = uc.get_data();
        let mut d = st.dev.borrow_mut();
        let rs = pdsim_core::LCD_ROWSIZE;
        let mut changed = vec![];
        {
            let f = d.gfx.frame_mut();
            for y in 0..pdsim_core::LCD_HEIGHT {
                if data.len() == n && f.data[y * rs..(y + 1) * rs] != data[y * rs..(y + 1) * rs] {
                    f.data[y * rs..(y + 1) * rs].copy_from_slice(&data[y * rs..(y + 1) * rs]);
                    changed.push(y as i32);
                }
            }
        }
        for y in changed {
            d.gfx.mark_updated_rows(y, y);
        }
    }

    /// Copies the frame back into the mirror after a drawing call.
    pub fn after_gfx(uc: &mut Unicorn<'_, ArmState>) {
        if !uc.get_data().frame_mirror {
            return;
        }
        let data = uc.get_data().dev.borrow().gfx.frame().data.clone();
        let _ = uc.mem_write(FRAME_ADDR as u64, &data);
    }
}

impl Game for ArmGame {
    fn device(&self) -> &SharedDevice {
        &self.dev
    }

    fn frame(&mut self, script: Option<&InputScript>) -> Result<FrameStats, GameError> {
        if let Some(e) = self.dev.borrow().stopped.clone() {
            return Err(GameError(e));
        }
        self.dev.borrow_mut().begin_frame(script);
        if self.uc.get_data().frame_mirror {
            self.frame_to_emu();
        }
        let (f, ud) = self.uc.get_data().update_cb.unwrap_or((0, 0));
        let r = self.top_call(f, &[ud]);
        if self.uc.get_data().frame_mirror {
            self.frame_from_emu();
        }
        let st = self.uc.get_data_mut();
        st.sync();
        st.flush_profile();
        let heap = st.heap.bytes_in_use;
        if let Err(e) = r {
            self.dev.borrow_mut().stopped = Some(e.0.clone());
            return Err(e);
        }
        let stats = {
            let mut d = self.dev.borrow_mut();
            d.c_heap = heap;
            d.end_frame(0)
        };
        self.dev.borrow_mut().last_frame = Some(stats.clone());
        Ok(stats)
    }

    fn describe(&self) -> String {
        format!("C game {}", self.path)
    }
}

/// Reads a `PDRect`-style float quad from an argument.
pub fn rect_arg(a: Arg) -> [f32; 4] {
    match a {
        Arg::PdRect(r) => r,
        _ => [0.0; 4],
    }
}
