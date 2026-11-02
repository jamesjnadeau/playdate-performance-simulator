//! The `playdate.*` API.
//!
//! Rust natives (the `__pd` table) do the work and charge calibrated costs;
//! `lua/geometry.lua` and `lua/api.lua` build the public API on top of them.
//! Those two files are loaded as `=[pdsim]` chunks, so the VM does not
//! charge their instructions as game code.

pub mod gfx;
pub mod sprite;
pub mod sys;
pub mod util;

use crate::ctx::CtxRef;
use crate::vm::NATIVE_CHUNK_PREFIX;
use mlua::{Lua, Result, Table};
use pdsim_core::Category;
use util::reg;

const GEOMETRY_LUA: &str = include_str!("../../lua/geometry.lua");
const API_LUA: &str = include_str!("../../lua/api.lua");
const PATHFINDER_LUA: &str = include_str!("../../lua/pathfinder.lua");

/// Builds the `playdate` global and friends.
pub fn install(lua: &Lua, ctx: &CtxRef) -> Result<Table> {
    let pd = lua.create_table()?;
    gfx::register(lua, &pd, ctx)?;
    sprite::register(lua, &pd, ctx)?;
    sys::register(lua, &pd, ctx)?;
    reg(lua, &pd, "wait", ctx, |lua, c, a| {
        c.sync(lua);
        let ms = a.f(0, 0.0).max(0.0);
        let mut d = c.dev();
        let cycles = ms * d.cal.cpu_hz as f64 / 1000.0;
        d.charge(Category::Other, cycles);
        Ok(())
    })?;
    lua.globals().set("playdate", lua.create_table()?)?;
    lua.load(GEOMETRY_LUA)
        .set_name(format!("{NATIVE_CHUNK_PREFIX}geometry"))
        .call::<()>(pd.clone())?;
    lua.load(API_LUA)
        .set_name(format!("{NATIVE_CHUNK_PREFIX}api"))
        .call::<()>(pd.clone())?;
    lua.load(PATHFINDER_LUA)
        .set_name(format!("{NATIVE_CHUNK_PREFIX}pathfinder"))
        .call::<()>(pd.clone())?;
    let preprocess =
        lua.create_function(|_, s: mlua::LuaString| Ok(crate::preprocess::preprocess(&s.to_string_lossy())))?;
    lua.globals()
        .get::<Table>("playdate")?
        .set("__preprocess", preprocess)?;
    Ok(pd)
}
