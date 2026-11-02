//! Running a Lua game frame by frame.

use crate::api;
use crate::ctx::{Ctx, CtxRef};
use mlua::{Function, HookTriggers, Lua, Table, Value, VmState};
use pdsim_core::{CategoryCycles, FrameStats};
use pdsim_device::{Game, GameError, InputScript, SharedDevice};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Options for loading a Lua game.
#[derive(Debug, Clone)]
pub struct LuaOptions {
    /// Attribute cycles to Lua functions (hotspots / flame view).
    pub profile: bool,
    /// Sample the Lua stack every this many VM instructions.
    pub sample_interval: u32,
}

impl Default for LuaOptions {
    fn default() -> Self {
        LuaOptions {
            profile: true,
            sample_interval: 1000,
        }
    }
}

/// A Playdate Lua game loaded from its source folder.
pub struct LuaGame {
    lua: Lua,
    ctx: CtxRef,
    frame_fn: Function,
    main: String,
    /// Cycles spent running main.lua before the first frame.
    pub launch_cycles: CategoryCycles,
}

fn gerr(e: impl std::fmt::Display) -> GameError {
    GameError(e.to_string())
}

/// Finds the game's source folder: `dir` itself or `dir/Source`.
pub fn source_dir(dir: &Path) -> Option<PathBuf> {
    if dir.join("main.lua").exists() {
        return Some(dir.to_path_buf());
    }
    let s = dir.join("Source");
    s.join("main.lua").exists().then_some(s)
}

fn parse_pdxinfo(text: &str, lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            t.set(k.trim(), v.trim())?;
        }
    }
    Ok(t)
}

impl LuaGame {
    /// Loads the game whose source folder the device's file system points
    /// at, runs `main.lua`, and returns it ready for its first frame.
    pub fn load(dev: SharedDevice, opts: LuaOptions) -> Result<LuaGame, GameError> {
        let lua = Lua::new();
        let ctx = Ctx::new(dev.clone(), &lua);
        ctx.profiling.set(opts.profile);
        api::install(&lua, &ctx).map_err(gerr)?;
        let (game_dir, sdk) = {
            let d = dev.borrow();
            (d.fs.game_dir.clone(), d.sdk.clone())
        };
        let playdate: Table = lua.globals().get("playdate").map_err(gerr)?;
        if let Ok(info) = std::fs::read_to_string(game_dir.join("pdxinfo")) {
            playdate
                .set("metadata", parse_pdxinfo(&info, &lua).map_err(gerr)?)
                .map_err(gerr)?;
        } else {
            playdate
                .set("metadata", lua.create_table().map_err(gerr)?)
                .map_err(gerr)?;
        }
        install_import(&lua, &game_dir, sdk).map_err(gerr)?;
        if opts.profile {
            let c = ctx.clone();
            lua.set_hook(
                HookTriggers::new().every_nth_instruction(opts.sample_interval.max(1)),
                move |lua, _| {
                    c.sync(lua);
                    Ok(VmState::Continue)
                },
            )
            .map_err(gerr)?;
        }
        let import: Function = lua.globals().get("import").map_err(gerr)?;
        import.call::<()>("main").map_err(gerr)?;
        ctx.sync(&lua);
        let launch_cycles = {
            let mut d = dev.borrow_mut();
            d.profile = Default::default();
            d.ledger.take_frame()
        };
        let frame_fn: Function = playdate.get("__frame").map_err(gerr)?;
        Ok(LuaGame {
            lua,
            ctx,
            frame_fn,
            main: game_dir.join("main.lua").display().to_string(),
            launch_cycles,
        })
    }

    /// The Lua state (for tests and tools).
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    fn collect_dead(&self) {
        let bitmaps: Vec<u32> = std::mem::take(&mut *self.ctx.dead_bitmaps.borrow_mut());
        let sprites: Vec<u32> = std::mem::take(&mut *self.ctx.dead_sprites.borrow_mut());
        let mut d = self.ctx.dev();
        for b in bitmaps {
            d.gfx.free_bitmap(b);
        }
        for s in sprites {
            d.sprites.destroy(s);
        }
    }
}

impl Game for LuaGame {
    fn device(&self) -> &SharedDevice {
        &self.ctx.dev
    }

    fn frame(&mut self, script: Option<&InputScript>) -> Result<FrameStats, GameError> {
        if let Some(e) = self.ctx.dev.borrow().stopped.clone() {
            return Err(GameError(e));
        }
        self.ctx.dev().begin_frame(script);
        let r = self.frame_fn.call::<()>(());
        self.ctx.sync(&self.lua);
        self.collect_dead();
        if let Err(e) = r {
            let msg = e.to_string();
            self.ctx.dev().stopped = Some(msg.clone());
            return Err(GameError(msg));
        }
        let heap = self.lua.used_memory() as u64;
        let stats = self.ctx.dev().end_frame(heap);
        if let Ok(p) = self.lua.globals().get::<Table>("playdate") {
            let fps = if stats.frame_ms > 0.0 {
                1000.0 / stats.frame_ms
            } else {
                0.0
            };
            let _ = p.raw_set("__fps", fps);
        }
        self.ctx.dev().last_frame = Some(stats.clone());
        Ok(stats)
    }

    fn describe(&self) -> String {
        format!("Lua game {}", self.main)
    }
}

/// Installs the global `import`: game files first (relative to the importing
/// file, then the source root), then CoreLibs from the SDK or the bundled
/// stand-ins. Each file runs once.
fn install_import(lua: &Lua, game_dir: &Path, sdk: Option<PathBuf>) -> mlua::Result<()> {
    let loaded: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(HashSet::new()));
    let game_dir = game_dir.to_path_buf();
    let import = lua.create_function(move |lua, name: String| {
        // pdc also accepts imports of assets (images, sounds, ...); at run
        // time those do nothing.
        if Path::new(&name)
            .extension()
            .is_some_and(|e| e != "lua" && e != "pdz")
        {
            return Ok(Value::Nil);
        }
        let name = name.trim_end_matches(".lua").trim_end_matches(".pdz").to_string();
        // Directory of the importing chunk, for relative imports.
        let caller_dir = lua
            .inspect_stack(1, |d| d.source().source.map(|s| s.to_string()))
            .flatten()
            .and_then(|s| s.strip_prefix('@').map(|s| s.to_string()))
            .and_then(|s| Path::new(&s).parent().map(|p| p.to_path_buf()))
            .unwrap_or_default();
        let mut candidates: Vec<(String, PathBuf)> = vec![];
        if !caller_dir.as_os_str().is_empty() {
            let rel = caller_dir.join(format!("{name}.lua"));
            candidates.push((rel.display().to_string(), game_dir.join(&rel)));
        }
        candidates.push((format!("{name}.lua"), game_dir.join(format!("{name}.lua"))));
        candidates.push((format!("{name}/main.lua"), game_dir.join(&name).join("main.lua")));
        let found = candidates.into_iter().find(|(_, p)| p.is_file());
        let (chunk_name, source) = if let Some((rel, path)) = found {
            (
                rel,
                std::fs::read_to_string(&path).map_err(mlua::Error::external)?,
            )
        } else if let Some(lib) = name.strip_prefix("CoreLibs/") {
            let from_sdk = sdk
                .as_ref()
                .map(|s| s.join("CoreLibs").join(format!("{lib}.lua")))
                .filter(|p| p.is_file());
            match from_sdk {
                Some(p) => (
                    format!("CoreLibs/{lib}.lua"),
                    std::fs::read_to_string(&p).map_err(mlua::Error::external)?,
                ),
                None => match crate::corelibs::fallback(lib) {
                    Some(s) => (format!("CoreLibs/{lib}.lua"), s.to_string()),
                    None => {
                        return Err(mlua::Error::runtime(format!(
                            "import: CoreLibs/{lib} not available"
                        )))
                    }
                },
            }
        } else {
            return Err(mlua::Error::runtime(format!("import: '{name}' not found")));
        };
        if !loaded.borrow_mut().insert(chunk_name.clone()) {
            return Ok(Value::Nil);
        }
        let code = crate::preprocess::preprocess(&source);
        lua.load(code)
            .set_name(format!("@{chunk_name}"))
            .call::<Value>(())
    })?;
    lua.globals().set("import", import)
}
