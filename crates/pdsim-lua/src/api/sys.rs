//! System, input, display, file and JSON natives.

use super::util::*;
use crate::ctx::CtxRef;
use mlua::{Lua, LuaSerdeExt, Result, Table, Value};
use pdsim_core::{Category, Work};

/// Device "wall clock" epoch: 2026-01-01T00:00:00Z. Time on the simulated
/// device advances with device time, so runs are reproducible.
pub const EPOCH: f64 = 1_767_225_600.0;

pub fn register(lua: &Lua, pd: &Table, ctx: &CtxRef) -> Result<()> {
    reg(lua, pd, "charge", ctx, |lua, c, a| {
        // (apiName, category, items, pixels, bytes) for API functions written in Lua.
        let cat = match a.str(1).as_deref() {
            Some("gfx") => Category::Gfx,
            Some("io") => Category::Io,
            Some("audio") => Category::Audio,
            _ => Category::Other,
        };
        let w = Work {
            items: a.f(2, 0.0) as u64,
            pixels: a.f(3, 0.0) as u64,
            bytes: a.f(4, 0.0) as u64,
            rows: 0,
        };
        c.api(lua, cat, &a.str(0).unwrap_or_default(), w);
        Ok(())
    })?;
    reg(lua, pd, "time_ms", ctx, |lua, c, _| {
        c.sync(lua);
        Ok(c.dev().now_ms().floor() as i64)
    })?;
    reg(lua, pd, "epoch", ctx, |lua, c, _| {
        c.sync(lua);
        let ms = c.dev().now_ms();
        let t = EPOCH + ms / 1000.0;
        Ok((t.floor() as i64, (t.fract() * 1000.0).floor() as i64))
    })?;
    reg(lua, pd, "elapsed", ctx, |lua, c, _| {
        c.sync(lua);
        Ok(c.dev().elapsed_s_sample())
    })?;
    reg(lua, pd, "reset_elapsed", ctx, |lua, c, _| {
        c.sync(lua);
        c.dev().reset_elapsed();
        Ok(())
    })?;
    reg(lua, pd, "input", ctx, |_, c, _| {
        let d = c.dev();
        Ok((d.input.current, d.input.pushed, d.input.released))
    })?;
    reg(lua, pd, "crank", ctx, |_, c, _| {
        let d = c.dev();
        Ok((d.input.crank_angle, d.input.crank_change, d.input.crank_docked))
    })?;
    reg(lua, pd, "accel", ctx, |_, c, _| {
        let d = c.dev();
        Ok(d.input.accel)
    })?;
    reg(lua, pd, "log", ctx, |_, c, a| {
        let s = a.str(0).unwrap_or_default();
        c.dev().log_line(s);
        Ok(())
    })?;
    reg(lua, pd, "frame", ctx, |_, c, _| {
        Ok(c.dev().scheduler.frame_index())
    })?;
    reg(lua, pd, "set_refresh", ctx, |_, c, a| {
        c.dev().scheduler.set_refresh_rate(a.f(0, 30.0) as f32);
        Ok(())
    })?;
    reg(lua, pd, "get_refresh", ctx, |_, c, _| {
        Ok(c.dev().scheduler.refresh_rate())
    })?;
    reg(lua, pd, "display_set", ctx, |_, c, a| {
        let mut d = c.dev();
        match a.str(0).as_deref() {
            Some("inverted") => d.gfx.inverted = a.bool(1, false),
            Some("scale") => d.gfx.scale = a.i(1, 1).clamp(1, 8),
            Some("offset") => d.gfx.display_offset = (a.i(1, 0), a.i(2, 0)),
            _ => {}
        }
        Ok(())
    })?;
    reg(lua, pd, "display_get", ctx, |_, c, a| {
        let d = c.dev();
        Ok(match a.str(0).as_deref() {
            Some("inverted") => (Value::Boolean(d.gfx.inverted), Value::Nil),
            Some("scale") => (Value::Integer(d.gfx.scale as i64), Value::Nil),
            Some("offset") => (
                Value::Integer(d.gfx.display_offset.0 as i64),
                Value::Integer(d.gfx.display_offset.1 as i64),
            ),
            _ => (Value::Nil, Value::Nil),
        })
    })?;
    reg(lua, pd, "stats", ctx, |lua, c, _| {
        // Percentages of the last frame, like playdate.getStats().
        let d = c.dev();
        let Some(f) = d.last_frame.as_ref() else {
            return Ok(Value::Nil);
        };
        let t = lua.create_table()?;
        let total = f.frame_ms.max(1e-9);
        let pct = |ms: f64| (ms / total * 100.0).round();
        let get = |k: Category| f.categories.get(&k).copied().unwrap_or(0.0);
        t.raw_set(
            "game",
            pct(get(Category::Lua)
                + get(Category::C)
                + get(Category::Gfx)
                + get(Category::Other)
                + get(Category::Io)),
        )?;
        t.raw_set("GC", pct(get(Category::Gc)))?;
        t.raw_set("audio", pct(get(Category::Audio)))?;
        t.raw_set("kernel", pct(get(Category::Display)))?;
        t.raw_set("idle", pct(f.frame_ms - f.work_ms))?;
        Ok(Value::Table(t))
    })?;

    // ---- files (paths are relative to the game; writes go to the data folder)
    reg(lua, pd, "file_read", ctx, |lua, c, a| {
        let p = a.str(0).unwrap_or_default();
        let data = c.dev().fs.read(&p);
        let n = data.as_ref().map_or(0, |d| d.len() as u64);
        c.api(
            lua,
            Category::Io,
            "file.read",
            Work {
                bytes: n,
                ..Default::default()
            },
        );
        data.map(|d| lua.create_string(d)).transpose()
    })?;
    reg(lua, pd, "file_write", ctx, |lua, c, a| {
        let p = a.str(0).unwrap_or_default();
        let data = match a.get(1) {
            Value::String(s) => s.as_bytes().to_vec(),
            _ => vec![],
        };
        let append = a.bool(2, false);
        let ok = {
            let d = c.dev();
            if append {
                let mut cur = d.fs.read(&p).unwrap_or_default();
                cur.extend_from_slice(&data);
                d.fs.write(&p, &cur)
            } else {
                d.fs.write(&p, &data)
            }
        };
        c.api(
            lua,
            Category::Io,
            "file.write",
            Work {
                bytes: data.len() as u64,
                ..Default::default()
            },
        );
        Ok(ok)
    })?;
    reg(lua, pd, "file_info", ctx, |_, c, a| {
        // exists, isDir, size, modtime (unix seconds)
        let p = a.str(0).unwrap_or_default();
        let d = c.dev();
        let Some(path) = d.fs.resolve_read(&p) else {
            return Ok((false, false, 0u64, 0u64));
        };
        let md = std::fs::metadata(&path).ok();
        let size = md.as_ref().map_or(0, |m| m.len());
        let mt = md
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        Ok((true, path.is_dir(), size, mt))
    })?;
    reg(lua, pd, "file_list", ctx, |lua, c, a| {
        let v = c.dev().fs.list(&a.str(0).unwrap_or_default());
        lua.create_sequence_from(v)
    })?;
    reg(lua, pd, "file_delete", ctx, |_, c, a| {
        Ok(c.dev().fs.delete(&a.str(0).unwrap_or_default()))
    })?;
    reg(lua, pd, "file_mkdir", ctx, |_, c, a| {
        let d = c.dev();
        Ok(pdsim_device::FileSystem::clean(&a.str(0).unwrap_or_default())
            .is_some_and(|p| std::fs::create_dir_all(d.fs.data_dir.join(p)).is_ok()))
    })?;
    reg(lua, pd, "file_rename", ctx, |_, c, a| {
        let d = c.dev();
        let (Some(f), Some(t)) = (
            pdsim_device::FileSystem::clean(&a.str(0).unwrap_or_default()),
            pdsim_device::FileSystem::clean(&a.str(1).unwrap_or_default()),
        ) else {
            return Ok(false);
        };
        Ok(std::fs::rename(d.fs.data_dir.join(f), d.fs.data_dir.join(t)).is_ok())
    })?;

    // ---- json
    reg_raw(lua, pd, "json_encode", ctx, |lua, c, a| {
        let v: serde_json::Value = lua.from_value(a.get(0).clone())?;
        let s = if a.bool(1, false) {
            serde_json::to_string_pretty(&v)
        } else {
            serde_json::to_string(&v)
        }
        .map_err(mlua::Error::external)?;
        c.api(
            lua,
            Category::Other,
            "json.encode",
            Work {
                bytes: s.len() as u64,
                ..Default::default()
            },
        );
        Ok(s)
    })?;
    reg(lua, pd, "json_decode", ctx, |lua, c, a| {
        let s = a.str(0).unwrap_or_default();
        c.api(
            lua,
            Category::Other,
            "json.decode",
            Work {
                bytes: s.len() as u64,
                ..Default::default()
            },
        );
        match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(v) => Ok((lua.to_value(&v)?, Value::Nil)),
            Err(e) => Ok((Value::Nil, Value::String(lua.create_string(e.to_string())?))),
        }
    })?;
    Ok(())
}
