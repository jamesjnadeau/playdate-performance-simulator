//! Sprite natives (`__pd.s_*`). `playdate.graphics.sprite` is built on these
//! in `lua/api.lua`.

use super::util::*;
use crate::ctx::CtxRef;
use mlua::{Function, Lua, Result, Table, Value};
use pdsim_core::{Category, Work};
use pdsim_gfx::sprite::{CollisionResponse, DrawItem, RectF};
use pdsim_gfx::*;

fn ids(lua: &Lua, v: Vec<SpriteId>) -> Result<Table> {
    lua.create_sequence_from(v)
}

pub fn register(lua: &Lua, pd: &Table, ctx: &CtxRef) -> Result<()> {
    reg(lua, pd, "s_new", ctx, |_, c, _| Ok(c.dev().sprites.create()))?;
    reg(lua, pd, "s_free", ctx, |_, c, a| {
        if let Some(id) = a.num(0) {
            c.dead_sprites.borrow_mut().push(id as u32);
        }
        Ok(())
    })?;
    reg(lua, pd, "s_add", ctx, |lua, c, a| {
        c.dev().sprites.add(a.i(0, 0) as u32);
        c.api(lua, Category::Gfx, "sprite.add", Work::default());
        Ok(())
    })?;
    reg(lua, pd, "s_remove", ctx, |lua, c, a| {
        c.dev().sprites.remove(a.i(0, 0) as u32);
        c.api(lua, Category::Gfx, "sprite.remove", Work::default());
        Ok(())
    })?;
    reg(lua, pd, "s_remove_all", ctx, |_, c, _| {
        c.dev().sprites.remove_all();
        Ok(())
    })?;
    reg(lua, pd, "s_list", ctx, |lua, c, _| {
        let v = c.dev().sprites.display_list();
        ids(lua, v)
    })?;
    reg(lua, pd, "s_update_list", ctx, |lua, c, _| {
        let v = c.dev().sprites.update_list();
        ids(lua, v)
    })?;
    reg(lua, pd, "s_count", ctx, |_, c, _| Ok(c.dev().sprites.count()))?;
    reg(lua, pd, "s_set_image", ctx, |lua, c, a| {
        let id = a.i(0, 0) as u32;
        let img = a.num(1).map(|n| n as u32);
        let mut d = c.dev();
        let size = img.and_then(|i| d.gfx.bitmap(i).map(|b| (b.width as f32, b.height as f32)));
        d.sprites.modify(id, |s| {
            s.image = img;
            if let Some((w, h)) = size {
                s.width = w;
                s.height = h;
            }
        });
        let (w, h) = d.sprites.get(id).map_or((0.0, 0.0), |s| (s.width, s.height));
        drop(d);
        c.api(lua, Category::Gfx, "sprite.setImage", Work::default());
        Ok((w, h))
    })?;
    reg(lua, pd, "s_move", ctx, |lua, c, a| {
        let (x, y) = (a.f(1, 0.0) as f32, a.f(2, 0.0) as f32);
        c.dev().sprites.modify(a.i(0, 0) as u32, |s| {
            s.x = x;
            s.y = y;
        });
        c.api(lua, Category::Gfx, "sprite.moveTo", Work::default());
        Ok(())
    })?;
    reg(lua, pd, "s_set_size", ctx, |_, c, a| {
        let (w, h) = (a.f(1, 0.0) as f32, a.f(2, 0.0) as f32);
        c.dev().sprites.modify(a.i(0, 0) as u32, |s| {
            s.width = w;
            s.height = h;
        });
        Ok(())
    })?;
    reg(lua, pd, "s_set_center", ctx, |_, c, a| {
        let (x, y) = (a.f(1, 0.5) as f32, a.f(2, 0.5) as f32);
        c.dev().sprites.modify(a.i(0, 0) as u32, |s| s.center = (x, y));
        Ok(())
    })?;
    reg(lua, pd, "s_state", ctx, |_, c, a| {
        // x, y, width, height, centerX, centerY
        let d = c.dev();
        Ok(d.sprites
            .get(a.i(0, 0) as u32)
            .map_or((0.0, 0.0, 0.0, 0.0, 0.5, 0.5), |s| {
                (s.x, s.y, s.width, s.height, s.center.0, s.center.1)
            }))
    })?;
    reg(lua, pd, "s_bounds", ctx, |_, c, a| {
        let d = c.dev();
        let b = d
            .sprites
            .get(a.i(0, 0) as u32)
            .map(|s| s.bounds())
            .unwrap_or_default();
        Ok((b.x, b.y, b.w, b.h))
    })?;
    reg(lua, pd, "s_set", ctx, |_, c, a| {
        let key = a.str(1).unwrap_or_default();
        let v = a.get(2).clone();
        let num = a.num(2).unwrap_or(0.0);
        let flag = a.bool(2, false);
        c.dev().sprites.modify(a.i(0, 0) as u32, |s| match key.as_str() {
            "z" => s.z = (num as i32).clamp(-32768, 32767),
            "visible" => s.visible = flag,
            "updates" => s.updates_enabled = flag,
            "opaque" => s.opaque = flag,
            "ignoreOffset" => s.ignores_draw_offset = flag,
            "collisions" => s.collisions_enabled = flag,
            "groups" => s.groups = num as u32,
            "collidesWith" => s.collides_with = num as u32,
            "tag" => s.tag = num as u8,
            "drawMode" => {
                s.draw_mode = match &v {
                    Value::String(st) => DrawMode::from_name(&st.to_string_lossy()).unwrap_or_default(),
                    _ => DrawMode::from_index(num as i64),
                }
            }
            "flip" => {
                s.flip = match &v {
                    Value::String(st) => Flip::from_name(&st.to_string_lossy()),
                    _ => Flip::from_index(num as i64),
                }
            }
            "hasDraw" => s.has_draw_callback = flag,
            "redrawOnImage" => s.redraws_on_image_change = flag,
            "alwaysRedraw" => s.always_redraw = flag,
            _ => {}
        });
        Ok(())
    })?;
    reg(lua, pd, "s_get", ctx, |_, c, a| {
        let key = a.str(1).unwrap_or_default();
        let d = c.dev();
        let Some(s) = d.sprites.get(a.i(0, 0) as u32) else {
            return Ok(Value::Nil);
        };
        Ok(match key.as_str() {
            "z" => Value::Integer(s.z as i64),
            "visible" => Value::Boolean(s.visible),
            "updates" => Value::Boolean(s.updates_enabled),
            "opaque" => Value::Boolean(s.opaque),
            "ignoreOffset" => Value::Boolean(s.ignores_draw_offset),
            "collisions" => Value::Boolean(s.collisions_enabled),
            "groups" => Value::Integer(s.groups as i64),
            "collidesWith" => Value::Integer(s.collides_with as i64),
            "tag" => Value::Integer(s.tag as i64),
            "drawMode" => Value::Integer(s.draw_mode.index()),
            "flip" => Value::Integer(s.flip as i64),
            "image" => s.image.map_or(Value::Nil, |i| Value::Integer(i as i64)),
            "added" => Value::Boolean(s.added),
            _ => Value::Nil,
        })
    })?;
    reg(lua, pd, "s_set_collide", ctx, |_, c, a| {
        let r = RectF::new(
            a.f(1, 0.0) as f32,
            a.f(2, 0.0) as f32,
            a.f(3, 0.0) as f32,
            a.f(4, 0.0) as f32,
        );
        c.dev()
            .sprites
            .modify(a.i(0, 0) as u32, |s| s.collide_rect = Some(r));
        Ok(())
    })?;
    reg(lua, pd, "s_clear_collide", ctx, |_, c, a| {
        c.dev()
            .sprites
            .modify(a.i(0, 0) as u32, |s| s.collide_rect = None);
        Ok(())
    })?;
    reg(lua, pd, "s_collide", ctx, |_, c, a| {
        // local collide rect, then world collide rect
        let d = c.dev();
        let Some(s) = d.sprites.get(a.i(0, 0) as u32) else {
            return Ok(mlua::MultiValue::new());
        };
        let (Some(l), Some(w)) = (s.collide_rect, s.world_collide_rect()) else {
            return Ok(mlua::MultiValue::new());
        };
        Ok([l.x, l.y, l.w, l.h, w.x, w.y, w.w, w.h]
            .into_iter()
            .map(|v| Value::Number(v as f64))
            .collect())
    })?;
    reg(lua, pd, "s_set_clip", ctx, |_, c, a| {
        let r = a
            .num(1)
            .map(|_| Rect::new(a.i(1, 0), a.i(2, 0), a.i(3, 0), a.i(4, 0)));
        c.dev().sprites.modify(a.i(0, 0) as u32, |s| s.clip_rect = r);
        Ok(())
    })?;
    reg(lua, pd, "s_mark_dirty", ctx, |lua, c, a| {
        c.dev().sprites.mark_dirty(a.i(0, 0) as u32);
        c.api(lua, Category::Gfx, "sprite.markDirty", Work::default());
        Ok(())
    })?;
    reg(lua, pd, "s_dirty_rect", ctx, |lua, c, a| {
        c.dev()
            .sprites
            .add_dirty_rect(Rect::new(a.i(0, 0), a.i(1, 0), a.i(2, 0), a.i(3, 0)));
        c.api(lua, Category::Gfx, "sprite.addDirtyRect", Work::default());
        Ok(())
    })?;
    reg(lua, pd, "s_always_redraw", ctx, |_, c, a| {
        let mut d = c.dev();
        if !a.is_empty() {
            d.sprites.always_redraw = a.bool(0, false);
        }
        Ok(d.sprites.always_redraw)
    })?;
    reg(lua, pd, "s_overlapping", ctx, |lua, c, a| {
        let v = c.dev().sprites.overlapping(a.i(0, 0) as u32);
        c.api(
            lua,
            Category::Gfx,
            "sprite.overlappingSprites",
            Work {
                items: v.len() as u64,
                ..Default::default()
            },
        );
        ids(lua, v)
    })?;
    reg(lua, pd, "s_all_overlapping", ctx, |lua, c, _| {
        let v = c.dev().sprites.all_overlapping();
        let n = c.dev().sprites.count() as u64;
        c.api(
            lua,
            Category::Gfx,
            "sprite.overlappingSprites",
            Work {
                items: n,
                ..Default::default()
            },
        );
        ids(lua, v.into_iter().flat_map(|(a, b)| [a, b]).collect())
    })?;
    reg(lua, pd, "s_query_point", ctx, |lua, c, a| {
        let v = c
            .dev()
            .sprites
            .query_point(a.f(0, 0.0) as f32, a.f(1, 0.0) as f32);
        let n = c.dev().sprites.count() as u64;
        c.api(
            lua,
            Category::Gfx,
            "sprite.querySpritesAtPoint",
            Work {
                items: n,
                ..Default::default()
            },
        );
        ids(lua, v)
    })?;
    reg(lua, pd, "s_query_rect", ctx, |lua, c, a| {
        let r = RectF::new(
            a.f(0, 0.0) as f32,
            a.f(1, 0.0) as f32,
            a.f(2, 0.0) as f32,
            a.f(3, 0.0) as f32,
        );
        let v = c.dev().sprites.query_rect(r);
        let n = c.dev().sprites.count() as u64;
        c.api(
            lua,
            Category::Gfx,
            "sprite.querySpritesInRect",
            Work {
                items: n,
                ..Default::default()
            },
        );
        ids(lua, v)
    })?;
    reg(lua, pd, "s_query_line", ctx, |lua, c, a| {
        let v = c.dev().sprites.query_line(
            a.f(0, 0.0) as f32,
            a.f(1, 0.0) as f32,
            a.f(2, 0.0) as f32,
            a.f(3, 0.0) as f32,
        );
        let n = c.dev().sprites.count() as u64;
        c.api(
            lua,
            Category::Gfx,
            "sprite.querySpritesInRect",
            Work {
                items: n,
                ..Default::default()
            },
        );
        ids(lua, v)
    })?;
    reg(lua, pd, "s_move_candidates", ctx, |lua, c, a| {
        let v = c
            .dev()
            .sprites
            .move_candidates(a.i(0, 0) as u32, a.f(1, 0.0) as f32, a.f(2, 0.0) as f32);
        ids(lua, v)
    })?;
    reg_raw(lua, pd, "s_move_collide", ctx, |lua, c, a| {
        // (id, x, y, responses {[otherId] = type}, checkOnly)
        let id = a.i(0, 0) as u32;
        let (x, y) = (a.f(1, 0.0) as f32, a.f(2, 0.0) as f32);
        let resp = a.table(3);
        let check = a.bool(4, false);
        let lookup = |o: SpriteId| -> CollisionResponse {
            resp.as_ref()
                .and_then(|t| t.raw_get::<Option<i64>>(o).ok().flatten())
                .map(CollisionResponse::from_index)
                .unwrap_or_default()
        };
        let (ax, ay, cols, n) = {
            let mut d = c.dev();
            let n = d.sprites.count() as u64;
            let (ax, ay, cols) = if check {
                d.sprites.check_collisions(id, x, y, &lookup)
            } else {
                d.sprites.move_with_collisions(id, x, y, &lookup)
            };
            (ax, ay, cols, n)
        };
        let out = lua.create_table()?;
        for (i, col) in cols.iter().enumerate() {
            let t = lua.create_table()?;
            t.raw_set("other", col.other)?;
            t.raw_set("type", col.response.index())?;
            t.raw_set("overlaps", col.overlaps)?;
            t.raw_set("ti", col.ti)?;
            t.raw_set("mx", col.move_x)?;
            t.raw_set("my", col.move_y)?;
            t.raw_set("nx", col.normal_x)?;
            t.raw_set("ny", col.normal_y)?;
            t.raw_set("tx", col.touch_x)?;
            t.raw_set("ty", col.touch_y)?;
            let r = |r: &RectF| lua.create_sequence_from([r.x, r.y, r.w, r.h]);
            t.raw_set("srect", r(&col.sprite_rect)?)?;
            t.raw_set("orect", r(&col.other_rect)?)?;
            if let Some((bx, by)) = col.bounce {
                t.raw_set("bx", bx)?;
                t.raw_set("by", by)?;
            }
            if let Some((sx, sy)) = col.slide {
                t.raw_set("sx", sx)?;
                t.raw_set("sy", sy)?;
            }
            out.raw_set(i + 1, t)?;
        }
        c.api(
            lua,
            Category::Gfx,
            "sprite.moveWithCollisions",
            Work {
                items: n,
                ..Default::default()
            },
        );
        Ok((ax, ay, out))
    })?;
    reg_raw(lua, pd, "s_draw", ctx, |lua, c, a| {
        // Draws the sprites that need it. `cb(id, x, y, w, h)` runs a
        // sprite's draw callback.
        let cb: Option<Function> = match a.get(0) {
            Value::Function(f) => Some(f.clone()),
            _ => None,
        };
        let (passes, saved, count) = {
            let mut d = c.dev();
            let off = d.gfx.state.draw_offset;
            let count = d.sprites.count() as u64;
            (d.sprites.plan_draw(off), d.gfx.state.clone(), count)
        };
        let mut work = Work::default();
        for pass in &passes {
            {
                let mut d = c.dev();
                let font = d.gfx.state.font;
                d.gfx.state = State {
                    font,
                    clip: Some(pass.rect),
                    ..State::default()
                };
                let bg = d.gfx.background_color;
                if pass.fill_background && bg != Color::Clear {
                    work.add(d.gfx.fill_rect(pass.rect, bg));
                }
            }
            for item in &pass.items {
                match item {
                    DrawItem::Image {
                        image,
                        x,
                        y,
                        flip,
                        mode,
                        ..
                    } => {
                        let mut d = c.dev();
                        d.gfx.state.draw_offset = (0, 0);
                        d.gfx.state.clip = Some(pass.rect);
                        d.gfx.state.draw_mode = *mode;
                        work.add(d.gfx.draw_bitmap(*image, *x, *y, *flip));
                    }
                    DrawItem::Callback { sprite, x, y, local } => {
                        {
                            let mut d = c.dev();
                            let font = d.gfx.state.font;
                            d.gfx.state = State {
                                font,
                                clip: Some(pass.rect),
                                draw_offset: (*x, *y),
                                ..State::default()
                            };
                        }
                        if let Some(f) = &cb {
                            f.call::<()>((*sprite, local.x, local.y, local.w, local.h))?;
                        }
                    }
                }
            }
        }
        c.dev().gfx.state = saved;
        work.items = count;
        c.api(lua, Category::Gfx, "sprite.update", work);
        Ok(passes.len())
    })?;
    Ok(())
}
