//! Graphics natives (`__pd.g_*`). The public `playdate.graphics` API is
//! built on these in `lua/api.lua`.

use super::util::*;
use crate::ctx::CtxRef;
use mlua::{Lua, Result, Table, Value};
use pdsim_core::{Category, Work};
use pdsim_gfx::*;

const GFX: Category = Category::Gfx;

fn color(n: i32) -> Color {
    Color::from_index(n as i64)
}

fn wrap_mode(n: i32) -> WrapMode {
    match n {
        0 => WrapMode::Clip,
        1 => WrapMode::Character,
        _ => WrapMode::Word,
    }
}

fn align(n: i32) -> TextAlign {
    match n {
        1 => TextAlign::Right,
        2 => TextAlign::Center,
        _ => TextAlign::Left,
    }
}

/// Strips Playdate text markup (`*bold*`, `_italic_`; `**` and `__` are
/// literal characters).
pub fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '*' || c == '_' {
            if it.peek() == Some(&c) {
                out.push(c);
                it.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

pub fn register(lua: &Lua, pd: &Table, ctx: &CtxRef) -> Result<()> {
    // ---- state
    reg(lua, pd, "g_set_color", ctx, |_, c, a| {
        c.dev().gfx.state.color = color(a.i(0, 0));
        Ok(())
    })?;
    reg(lua, pd, "g_get_color", ctx, |_, c, _| {
        Ok(match c.dev().gfx.state.color {
            Color::Black => 0,
            Color::White => 1,
            Color::Clear => 2,
            Color::Xor => 3,
            Color::Pattern(_) => 0,
        })
    })?;
    reg(lua, pd, "g_set_pattern", ctx, |lua, c, a| {
        // 8 rows of bits, optionally followed by 8 rows of mask.
        let t = a
            .table(0)
            .ok_or_else(|| mlua::Error::runtime("setPattern: expected a table"))?;
        let mut bits = [0u8; 8];
        let mut mask = [0xffu8; 8];
        for i in 0..8 {
            bits[i] = t.raw_get::<Option<i64>>(i + 1)?.unwrap_or(0) as u8;
            if let Some(m) = t.raw_get::<Option<i64>>(i + 9)? {
                mask[i] = m as u8;
            }
        }
        c.dev().gfx.state.color = Color::Pattern(Pattern { bits, mask });
        c.api(lua, GFX, "graphics.setPattern", Work::default());
        Ok(())
    })?;
    reg(lua, pd, "g_set_pattern_image", ctx, |lua, c, a| {
        let (id, ox, oy) = (a.i(0, 0) as u32, a.i(1, 0), a.i(2, 0));
        let mut d = c.dev();
        if let Some(b) = d.gfx.bitmap(id).cloned() {
            let mut bits = [0u8; 8];
            let mut mask = [0u8; 8];
            for y in 0..8 {
                for x in 0..8 {
                    let (sx, sy) = (ox + x, oy + y);
                    if b.get(sx, sy) {
                        bits[y as usize] |= 0x80 >> x;
                    }
                    if b.opaque(sx, sy) {
                        mask[y as usize] |= 0x80 >> x;
                    }
                }
            }
            d.gfx.state.color = Color::Pattern(Pattern { bits, mask });
        }
        drop(d);
        c.api(lua, GFX, "graphics.setPattern", Work::default());
        Ok(())
    })?;
    reg(lua, pd, "g_set_dither", ctx, |lua, c, a| {
        let alpha = a.f(0, 0.5).clamp(0.0, 1.0) as f32;
        let mut d = c.dev();
        let white = matches!(d.gfx.state.color, Color::White);
        let p = if white {
            Pattern {
                bits: [0xff; 8],
                mask: Pattern::dither(1.0 - alpha).bits,
            }
        } else {
            Pattern {
                bits: [0; 8],
                mask: Pattern::dither(alpha).bits,
            }
        };
        d.gfx.state.color = Color::Pattern(p);
        drop(d);
        c.api(lua, GFX, "graphics.setDitherPattern", Work::default());
        Ok(())
    })?;
    reg(lua, pd, "g_set_bg", ctx, |_, c, a| {
        c.dev().gfx.background_color = color(a.i(0, 1));
        Ok(())
    })?;
    reg(lua, pd, "g_get_bg", ctx, |_, c, _| {
        Ok(match c.dev().gfx.background_color {
            Color::Black => 0,
            Color::Clear => 2,
            _ => 1,
        })
    })?;
    reg(lua, pd, "g_set_draw_mode", ctx, |_, c, a| {
        let m = match a.get(0) {
            Value::String(s) => DrawMode::from_name(&s.to_string_lossy()).unwrap_or_default(),
            _ => DrawMode::from_index(a.i(0, 0) as i64),
        };
        c.dev().gfx.state.draw_mode = m;
        Ok(())
    })?;
    reg(lua, pd, "g_get_draw_mode", ctx, |_, c, _| {
        Ok(c.dev().gfx.state.draw_mode.index())
    })?;
    reg(lua, pd, "g_set_line_width", ctx, |_, c, a| {
        c.dev().gfx.state.line_width = a.i(0, 1).max(0);
        Ok(())
    })?;
    reg(lua, pd, "g_get_line_width", ctx, |_, c, _| {
        Ok(c.dev().gfx.state.line_width)
    })?;
    reg(lua, pd, "g_set_line_cap", ctx, |_, c, a| {
        c.dev().gfx.state.line_cap = match a.i(0, 0) {
            1 => LineCap::Square,
            2 => LineCap::Round,
            _ => LineCap::Butt,
        };
        Ok(())
    })?;
    reg(lua, pd, "g_set_stroke", ctx, |_, c, a| {
        c.dev().gfx.state.stroke = match a.i(0, 0) {
            1 => StrokeLocation::Inside,
            2 => StrokeLocation::Outside,
            _ => StrokeLocation::Centered,
        };
        Ok(())
    })?;
    reg(lua, pd, "g_get_stroke", ctx, |_, c, _| {
        Ok(match c.dev().gfx.state.stroke {
            StrokeLocation::Centered => 0,
            StrokeLocation::Inside => 1,
            StrokeLocation::Outside => 2,
        })
    })?;
    reg(lua, pd, "g_set_poly_rule", ctx, |_, c, a| {
        c.dev().gfx.state.polygon_rule = if a.i(0, 0) == 0 {
            FillRule::NonZero
        } else {
            FillRule::EvenOdd
        };
        Ok(())
    })?;
    reg(lua, pd, "g_set_offset", ctx, |_, c, a| {
        c.dev().gfx.state.draw_offset = (a.i(0, 0), a.i(1, 0));
        Ok(())
    })?;
    reg(lua, pd, "g_get_offset", ctx, |_, c, _| {
        Ok(c.dev().gfx.state.draw_offset)
    })?;
    reg(lua, pd, "g_set_clip", ctx, |lua, c, a| {
        c.dev()
            .gfx
            .set_clip_rect(Rect::new(a.i(0, 0), a.i(1, 0), a.i(2, 0), a.i(3, 0)));
        c.api(lua, GFX, "graphics.setClipRect", Work::default());
        Ok(())
    })?;
    reg(lua, pd, "g_set_screen_clip", ctx, |lua, c, a| {
        c.dev()
            .gfx
            .set_screen_clip_rect(Rect::new(a.i(0, 0), a.i(1, 0), a.i(2, 0), a.i(3, 0)));
        c.api(lua, GFX, "graphics.setClipRect", Work::default());
        Ok(())
    })?;
    reg(lua, pd, "g_get_clip", ctx, |_, c, _| {
        let r = c.dev().gfx.clip_rect();
        Ok((r.x, r.y, r.w, r.h))
    })?;
    reg(lua, pd, "g_get_screen_clip", ctx, |_, c, _| {
        let d = c.dev();
        let r = d.gfx.state.clip.unwrap_or(Rect::new(0, 0, 400, 240));
        Ok((r.x, r.y, r.w, r.h))
    })?;
    reg(lua, pd, "g_clear_clip", ctx, |_, c, _| {
        c.dev().gfx.clear_clip_rect();
        Ok(())
    })?;
    reg(lua, pd, "g_set_stencil", ctx, |_, c, a| {
        let mut d = c.dev();
        d.gfx.state.stencil = a.num(0).map(|id| (id as u32, a.bool(1, false)));
        Ok(())
    })?;
    reg(lua, pd, "g_push_context", ctx, |lua, c, a| {
        c.dev().gfx.push_context(a.num(0).map(|n| n as u32));
        c.api(lua, GFX, "graphics.pushContext", Work::default());
        Ok(())
    })?;
    reg(lua, pd, "g_pop_context", ctx, |lua, c, _| {
        c.dev().gfx.pop_context();
        c.api(lua, GFX, "graphics.popContext", Work::default());
        Ok(())
    })?;
    reg(lua, pd, "g_target", ctx, |_, c, _| Ok(c.dev().gfx.state.target))?;

    // ---- shapes: each charges its calibrated cost for the work done
    macro_rules! shape {
        ($name:literal, $api:literal, |$g:ident, $a:ident| $body:expr) => {
            reg(lua, pd, $name, ctx, |lua, c, $a| {
                let w = {
                    let mut d = c.dev();
                    let $g = &mut d.gfx;
                    $body
                };
                c.api(lua, GFX, $api, w);
                Ok(())
            })?;
        };
    }
    shape!("g_clear", "graphics.clear", |g, a| g.clear(color(a.i(0, 1))));
    shape!("g_fill_rect", "graphics.fillRect", |g, a| {
        let col = g.state.color;
        g.fill_rect(Rect::new(a.i(0, 0), a.i(1, 0), a.i(2, 0), a.i(3, 0)), col)
    });
    shape!("g_draw_rect", "graphics.drawRect", |g, a| {
        let col = g.state.color;
        g.draw_rect(Rect::new(a.i(0, 0), a.i(1, 0), a.i(2, 0), a.i(3, 0)), col)
    });
    shape!("g_draw_line", "graphics.drawLine", |g, a| {
        let col = g.state.color;
        g.draw_line(a.i(0, 0), a.i(1, 0), a.i(2, 0), a.i(3, 0), col)
    });
    shape!("g_draw_pixel", "graphics.drawPixel", |g, a| {
        let col = g.state.color;
        g.draw_pixel(a.i(0, 0), a.i(1, 0), col)
    });
    shape!("g_fill_ellipse", "graphics.fillEllipse", |g, a| {
        let col = g.state.color;
        g.fill_ellipse(
            Rect::new(a.i(0, 0), a.i(1, 0), a.i(2, 0), a.i(3, 0)),
            a.f(4, 0.0) as f32,
            a.f(5, 0.0) as f32,
            col,
        )
    });
    shape!("g_draw_ellipse", "graphics.drawEllipse", |g, a| {
        let (col, lw) = (g.state.color, g.state.line_width);
        g.draw_ellipse(
            Rect::new(a.i(0, 0), a.i(1, 0), a.i(2, 0), a.i(3, 0)),
            lw,
            a.f(4, 0.0) as f32,
            a.f(5, 0.0) as f32,
            col,
        )
    });
    shape!("g_fill_round_rect", "graphics.fillRoundRect", |g, a| {
        let col = g.state.color;
        g.fill_round_rect(
            Rect::new(a.i(0, 0), a.i(1, 0), a.i(2, 0), a.i(3, 0)),
            a.i(4, 0),
            col,
        )
    });
    shape!("g_draw_round_rect", "graphics.drawRoundRect", |g, a| {
        let (col, lw) = (g.state.color, g.state.line_width);
        g.draw_round_rect(
            Rect::new(a.i(0, 0), a.i(1, 0), a.i(2, 0), a.i(3, 0)),
            a.i(4, 0),
            lw,
            col,
        )
    });
    shape!("g_fill_triangle", "graphics.fillTriangle", |g, a| {
        let col = g.state.color;
        g.fill_triangle(
            [
                (a.i(0, 0), a.i(1, 0)),
                (a.i(2, 0), a.i(3, 0)),
                (a.i(4, 0), a.i(5, 0)),
            ],
            col,
        )
    });
    // Polygons take a flat list of coordinates.
    fn poly(a: &Args) -> Vec<(i32, i32)> {
        let n = a.len() / 2;
        (0..n).map(|i| (a.i(2 * i, 0), a.i(2 * i + 1, 0))).collect()
    }
    shape!("g_fill_polygon", "graphics.fillPolygon", |g, a| {
        let (col, rule) = (g.state.color, g.state.polygon_rule);
        g.fill_polygon(&poly(&a), rule, col)
    });
    shape!("g_draw_polygon", "graphics.drawPolygon", |g, a| {
        // First argument: closed flag; then coordinates.
        let col = g.state.color;
        let closed = a.bool(0, true);
        g.draw_polygon(&poly(&a.rest(1)), closed, col)
    });

    // ---- bitmaps
    reg(lua, pd, "b_new", ctx, |lua, c, a| {
        let (w, h) = (a.i(0, 0), a.i(1, 0));
        let id = c.dev().gfx.new_bitmap(w, h, color(a.i(2, 2)));
        c.api(
            lua,
            GFX,
            "graphics.newBitmap",
            Work {
                bytes: ((w.max(0) as u64).div_ceil(8)) * h.max(0) as u64,
                ..Default::default()
            },
        );
        Ok(id)
    })?;
    reg(lua, pd, "b_load", ctx, |lua, c, a| {
        let path = a.str(0).unwrap_or_default();
        let found = c.dev().fs.resolve_with_ext(&path, &["png"]);
        let Some(p) = found else {
            return Ok((None, Some(format!("file not found: {path}"))));
        };
        let r = c.dev().gfx.load_bitmap(&p);
        match r {
            Ok(id) => {
                let bytes = c.dev().gfx.bitmap(id).map_or(0, |b| b.memory_bytes());
                c.api(
                    lua,
                    Category::Io,
                    "graphics.loadBitmap",
                    Work {
                        bytes,
                        ..Default::default()
                    },
                );
                Ok((Some(id), None))
            }
            Err(e) => Ok((None, Some(e.to_string()))),
        }
    })?;
    reg(lua, pd, "b_free", ctx, |_, c, a| {
        if let Some(id) = a.num(0) {
            c.dead_bitmaps.borrow_mut().push(id as u32);
        }
        Ok(())
    })?;
    reg(lua, pd, "b_size", ctx, |_, c, a| {
        let d = c.dev();
        Ok(d.gfx
            .bitmap(a.i(0, 0) as u32)
            .map_or((0, 0), |b| (b.width, b.height)))
    })?;
    reg(lua, pd, "b_draw", ctx, |lua, c, a| {
        let w = c.dev().gfx.draw_bitmap(
            a.i(0, 0) as u32,
            a.i(1, 0),
            a.i(2, 0),
            Flip::from_index(a.i(3, 0) as i64),
        );
        c.api(lua, GFX, "graphics.drawBitmap", w);
        Ok(())
    })?;
    reg(lua, pd, "b_draw_scaled", ctx, |lua, c, a| {
        let w = c.dev().gfx.draw_scaled_bitmap(
            a.i(0, 0) as u32,
            a.i(1, 0),
            a.i(2, 0),
            a.f(3, 1.0) as f32,
            a.f(4, a.f(3, 1.0)) as f32,
        );
        c.api(lua, GFX, "graphics.drawScaledBitmap", w);
        Ok(())
    })?;
    reg(lua, pd, "b_draw_rotated", ctx, |lua, c, a| {
        let w = c.dev().gfx.draw_rotated_bitmap(
            a.i(0, 0) as u32,
            a.i(1, 0),
            a.i(2, 0),
            a.f(3, 0.0) as f32,
            a.f(4, 0.5) as f32,
            a.f(5, 0.5) as f32,
            a.f(6, 1.0) as f32,
            a.f(7, a.f(6, 1.0)) as f32,
        );
        c.api(lua, GFX, "graphics.drawRotatedBitmap", w);
        Ok(())
    })?;
    reg(lua, pd, "b_tile", ctx, |lua, c, a| {
        let w = c.dev().gfx.tile_bitmap(
            a.i(0, 0) as u32,
            Rect::new(a.i(1, 0), a.i(2, 0), a.i(3, 0), a.i(4, 0)),
            Flip::from_index(a.i(5, 0) as i64),
        );
        c.api(lua, GFX, "graphics.tileBitmap", w);
        Ok(())
    })?;
    reg(lua, pd, "b_copy", ctx, |lua, c, a| {
        let id = c.dev().gfx.copy_bitmap(a.i(0, 0) as u32);
        let bytes = id
            .and_then(|i| c.dev().gfx.bitmap(i).map(|b| b.memory_bytes()))
            .unwrap_or(0);
        c.api(
            lua,
            GFX,
            "graphics.copyBitmap",
            Work {
                bytes,
                ..Default::default()
            },
        );
        Ok(id)
    })?;
    reg(lua, pd, "b_clear", ctx, |lua, c, a| {
        let mut d = c.dev();
        let id = a.i(0, 0) as u32;
        let col = color(a.i(1, 2));
        let w = if let Some(b) = d.gfx.bitmap_mut(id) {
            let had_mask = b.mask.is_some();
            b.clear(col);
            if had_mask && b.mask.is_none() {
                let n = b.data.len();
                b.mask = Some(vec![0xff; n]);
            }
            Work::pixels((b.width * b.height) as u64, b.height as u64)
        } else {
            Work::default()
        };
        if id == FRAME {
            d.gfx.mark_updated_rows(0, 239);
        }
        drop(d);
        c.api(lua, GFX, "graphics.clear", w);
        Ok(())
    })?;
    reg(lua, pd, "b_sample", ctx, |_, c, a| {
        let d = c.dev();
        let (x, y) = (a.i(1, 0), a.i(2, 0));
        Ok(match d.gfx.bitmap(a.i(0, 0) as u32) {
            Some(b) if !b.opaque(x, y) => 2,
            Some(b) if b.get(x, y) => 1,
            _ => 0,
        })
    })?;
    reg(lua, pd, "b_set_pixel", ctx, |_, c, a| {
        let mut d = c.dev();
        let (x, y) = (a.i(1, 0), a.i(2, 0));
        if let Some(b) = d.gfx.bitmap_mut(a.i(0, 0) as u32) {
            match a.i(3, 0) {
                2 => b.clear_pixel(x, y),
                n => b.set(x, y, n == 1),
            }
        }
        Ok(())
    })?;
    reg(lua, pd, "b_rotated", ctx, |lua, c, a| {
        let id = c.dev().gfx.rotated_bitmap(
            a.i(0, 0) as u32,
            a.f(1, 0.0) as f32,
            a.f(2, 1.0) as f32,
            a.f(3, a.f(2, 1.0)) as f32,
        );
        let px = id
            .and_then(|i| c.dev().gfx.bitmap(i).map(|b| (b.width * b.height) as u64))
            .unwrap_or(0);
        c.api(lua, GFX, "graphics.drawRotatedBitmap", Work::pixels(px, 0));
        Ok(id)
    })?;
    reg(lua, pd, "b_scaled", ctx, |lua, c, a| {
        let id = c
            .dev()
            .gfx
            .scaled_bitmap(a.i(0, 0) as u32, a.f(1, 1.0) as f32, a.f(2, a.f(1, 1.0)) as f32);
        let px = id
            .and_then(|i| c.dev().gfx.bitmap(i).map(|b| (b.width * b.height) as u64))
            .unwrap_or(0);
        c.api(lua, GFX, "graphics.drawScaledBitmap", Work::pixels(px, 0));
        Ok(id)
    })?;
    reg(lua, pd, "b_inverted", ctx, |lua, c, a| {
        let mut d = c.dev();
        let Some(mut b) = d.gfx.bitmap(a.i(0, 0) as u32).cloned() else {
            return Ok(None);
        };
        for v in b.data.iter_mut() {
            *v = !*v;
        }
        let px = (b.width * b.height) as u64;
        let id = d.gfx.add_bitmap(b);
        drop(d);
        c.api(lua, GFX, "graphics.copyBitmap", Work::pixels(px, 0));
        Ok(Some(id))
    })?;
    reg(lua, pd, "b_faded", ctx, |lua, c, a| {
        // Image with only `alpha` of its opaque pixels kept (dithered).
        let mut d = c.dev();
        let Some(mut b) = d.gfx.bitmap(a.i(0, 0) as u32).cloned() else {
            return Ok(None);
        };
        let p = Pattern::dither(a.f(1, 1.0) as f32);
        let n = b.data.len();
        let mut mask = b.mask.clone().unwrap_or_else(|| vec![0xff; n]);
        for y in 0..b.height {
            for x in 0..b.width {
                if p.at(x, y) != Some(true) {
                    let i = y as usize * b.rowbytes + (x as usize >> 3);
                    mask[i] &= !(0x80u8 >> (x & 7));
                }
            }
        }
        b.mask = Some(mask);
        let px = (b.width * b.height) as u64;
        let id = d.gfx.add_bitmap(b);
        drop(d);
        c.api(lua, GFX, "graphics.copyBitmap", Work::pixels(px, 0));
        Ok(Some(id))
    })?;
    reg(lua, pd, "b_set_mask", ctx, |_, c, a| {
        let mut d = c.dev();
        let Some(m) = d.gfx.bitmap(a.i(1, 0) as u32).cloned() else {
            return Ok(());
        };
        if let Some(b) = d.gfx.bitmap_mut(a.i(0, 0) as u32) {
            let mut mask = vec![0u8; b.data.len()];
            for y in 0..b.height {
                for x in 0..b.width {
                    if m.get(x, y) {
                        mask[y as usize * b.rowbytes + (x as usize >> 3)] |= 0x80 >> (x & 7);
                    }
                }
            }
            b.mask = Some(mask);
        }
        Ok(())
    })?;
    reg(lua, pd, "b_get_mask", ctx, |_, c, a| {
        let mut d = c.dev();
        let Some(b) = d.gfx.bitmap(a.i(0, 0) as u32).cloned() else {
            return Ok(None);
        };
        let mut m = Bitmap::new(b.width, b.height, Color::Black);
        for y in 0..b.height {
            for x in 0..b.width {
                m.set(x, y, b.opaque(x, y));
            }
        }
        Ok(Some(d.gfx.add_bitmap(m)))
    })?;
    reg(lua, pd, "b_add_mask", ctx, |_, c, a| {
        let mut d = c.dev();
        let opaque = a.bool(1, true);
        if let Some(b) = d.gfx.bitmap_mut(a.i(0, 0) as u32) {
            if b.mask.is_none() {
                b.mask = Some(vec![if opaque { 0xff } else { 0 }; b.data.len()]);
            }
        }
        Ok(())
    })?;
    reg(lua, pd, "b_remove_mask", ctx, |_, c, a| {
        if let Some(b) = c.dev().gfx.bitmap_mut(a.i(0, 0) as u32) {
            b.mask = None;
        }
        Ok(())
    })?;
    reg(lua, pd, "b_has_mask", ctx, |_, c, a| {
        Ok(c.dev()
            .gfx
            .bitmap(a.i(0, 0) as u32)
            .is_some_and(|b| b.mask.is_some()))
    })?;
    reg(lua, pd, "b_working", ctx, |_, c, _| {
        Ok(c.dev().gfx.copy_bitmap(FRAME))
    })?;
    reg(lua, pd, "b_display", ctx, |_, c, _| {
        let mut d = c.dev();
        let b = d.gfx.display().clone();
        Ok(d.gfx.add_bitmap(b))
    })?;
    reg(lua, pd, "b_alpha_collision", ctx, |lua, c, a| {
        // (id1, x1, y1, flip1, id2, x2, y2, flip2)
        let d = c.dev();
        let (Some(b1), Some(b2)) = (d.gfx.bitmap(a.i(0, 0) as u32), d.gfx.bitmap(a.i(4, 0) as u32)) else {
            return Ok(false);
        };
        let r1 = Rect::new(a.i(1, 0), a.i(2, 0), b1.width, b1.height);
        let r2 = Rect::new(a.i(5, 0), a.i(6, 0), b2.width, b2.height);
        let i = r1.intersect(&r2);
        let (f1, f2) = (
            Flip::from_index(a.i(3, 0) as i64),
            Flip::from_index(a.i(7, 0) as i64),
        );
        let px = |b: &Bitmap, r: Rect, f: Flip, x: i32, y: i32| {
            let mut sx = x - r.x;
            let mut sy = y - r.y;
            if f.x() {
                sx = b.width - 1 - sx;
            }
            if f.y() {
                sy = b.height - 1 - sy;
            }
            b.opaque(sx, sy)
        };
        let mut hit = false;
        'o: for y in i.y..i.y + i.h {
            for x in i.x..i.x + i.w {
                if px(b1, r1, f1, x, y) && px(b2, r2, f2, x, y) {
                    hit = true;
                    break 'o;
                }
            }
        }
        drop(d);
        c.api(
            lua,
            GFX,
            "graphics.checkMaskCollision",
            Work::pixels((i.w * i.h).max(0) as u64, 0),
        );
        Ok(hit)
    })?;

    // ---- image tables
    reg(lua, pd, "t_load", ctx, |lua, c, a| {
        let path = a.str(0).unwrap_or_default();
        let base = {
            let d = c.dev();
            let clean = pdsim_device::FileSystem::clean(&path).unwrap_or_default();
            let g = d.fs.game_dir.join(&clean);
            let dd = d.fs.data_dir.join(&clean);
            if dd.parent().is_some_and(|p| p.exists()) && pdsim_gfx::table::load_table(&dd).is_ok() {
                dd
            } else {
                g
            }
        };
        match pdsim_gfx::table::load_table(&base) {
            Ok(v) => {
                let t = lua.create_table()?;
                let mut bytes = 0;
                for (i, b) in v.into_iter().enumerate() {
                    bytes += b.memory_bytes();
                    t.raw_set(i + 1, c.dev().gfx.add_bitmap(b))?;
                }
                c.api(
                    lua,
                    Category::Io,
                    "graphics.loadBitmap",
                    Work {
                        bytes,
                        ..Default::default()
                    },
                );
                Ok((Some(t), None))
            }
            Err(e) => Ok((None, Some(e.to_string()))),
        }
    })?;

    // ---- fonts and text
    reg(lua, pd, "f_system", ctx, |_, c, _| Ok(c.dev().gfx.system_font()))?;
    reg(lua, pd, "f_current", ctx, |_, c, _| {
        Ok(c.dev().gfx.current_font())
    })?;
    reg(lua, pd, "f_set", ctx, |_, c, a| {
        c.dev().gfx.state.font = a.num(0).map(|n| n as u32);
        Ok(())
    })?;
    reg(lua, pd, "f_load", ctx, |lua, c, a| {
        let path = a.str(0).unwrap_or_default();
        let p = c.dev().fs.resolve_with_ext(&path, &["fnt"]);
        let Some(p) = p else {
            return Ok((None, Some(format!("font not found: {path}"))));
        };
        let r = c.dev().gfx.load_font(&p);
        match r {
            Ok(id) => {
                let bytes = c.dev().gfx.font(id).map_or(0, |f| f.memory_bytes());
                c.api(
                    lua,
                    Category::Io,
                    "graphics.loadFont",
                    Work {
                        bytes,
                        ..Default::default()
                    },
                );
                Ok((Some(id), None))
            }
            Err(e) => Ok((None, Some(e.to_string()))),
        }
    })?;
    reg(lua, pd, "f_height", ctx, |_, c, a| {
        Ok(c.dev().gfx.font(a.i(0, 0) as u32).map_or(0, |f| f.height()))
    })?;
    reg(lua, pd, "f_tracking", ctx, |_, c, a| {
        Ok(c.dev().gfx.font(a.i(0, 0) as u32).map_or(0, |f| f.tracking))
    })?;
    reg(lua, pd, "f_set_tracking", ctx, |_, c, a| {
        c.dev().gfx.set_font_tracking(a.i(0, 0) as u32, a.i(1, 0));
        Ok(())
    })?;
    reg(lua, pd, "g_set_tracking", ctx, |_, c, a| {
        c.dev().gfx.state.tracking = a.num(0).map(|n| n as i32);
        Ok(())
    })?;
    reg(lua, pd, "g_get_tracking", ctx, |_, c, _| {
        let d = c.dev();
        let f = d.gfx.current_font();
        Ok(d.gfx
            .state
            .tracking
            .unwrap_or_else(|| d.gfx.font(f).map_or(0, |f| f.tracking)))
    })?;
    reg(lua, pd, "g_set_leading", ctx, |_, c, a| {
        c.dev().gfx.state.leading = a.i(0, 0);
        Ok(())
    })?;
    reg(lua, pd, "f_text_width", ctx, |lua, c, a| {
        let text = strip_markup(&a.str(1).unwrap_or_default());
        let w = {
            let d = c.dev();
            let fid = a.i(0, 0) as u32;
            let tr = d.gfx.font(fid).map_or(0, |f| f.tracking);
            d.gfx.font(fid).map_or(0, |f| {
                text.split('\n').map(|l| f.line_width(l, tr)).max().unwrap_or(0)
            })
        };
        c.api(
            lua,
            GFX,
            "graphics.getTextSize",
            Work {
                items: text.chars().count() as u64,
                ..Default::default()
            },
        );
        Ok(w)
    })?;
    reg(lua, pd, "f_text_size", ctx, |lua, c, a| {
        let text = strip_markup(&a.str(1).unwrap_or_default());
        let (w, h) = c.dev().gfx.text_size(a.i(0, 0) as u32, &text);
        c.api(
            lua,
            GFX,
            "graphics.getTextSize",
            Work {
                items: text.chars().count() as u64,
                ..Default::default()
            },
        );
        Ok((w, h))
    })?;
    reg(lua, pd, "f_draw_text", ctx, |lua, c, a| {
        // (font, text, x, y)
        let text = strip_markup(&a.str(1).unwrap_or_default());
        let (w, h, work) = c
            .dev()
            .gfx
            .draw_text(a.i(0, 0) as u32, &text, a.i(2, 0), a.i(3, 0));
        c.api(lua, GFX, "graphics.drawText", work);
        Ok((w, h))
    })?;
    reg(lua, pd, "f_draw_text_in_rect", ctx, |lua, c, a| {
        // (font, text, x, y, w, h, leading, wrap, align)
        let text = strip_markup(&a.str(1).unwrap_or_default());
        let (layout, work) = {
            let mut d = c.dev();
            let saved = d.gfx.state.leading;
            d.gfx.state.leading = a.i(6, saved);
            let r = d.gfx.draw_text_in_rect(
                a.i(0, 0) as u32,
                &text,
                Rect::new(a.i(2, 0), a.i(3, 0), a.i(4, 0), a.i(5, 0)),
                wrap_mode(a.i(7, 2)),
                align(a.i(8, 0)),
            );
            d.gfx.state.leading = saved;
            r
        };
        c.api(lua, GFX, "graphics.drawText", work);
        Ok((layout.width, layout.height, layout.truncated))
    })?;
    reg(lua, pd, "f_text_size_max_width", ctx, |lua, c, a| {
        // (font, text, maxWidth, leading, wrap)
        let text = strip_markup(&a.str(1).unwrap_or_default());
        let (w, h) = {
            let d = c.dev();
            let fid = a.i(0, 0) as u32;
            let lines = d.gfx.wrap_text(fid, &text, a.i(2, 10_000), wrap_mode(a.i(4, 2)));
            let f = d.gfx.font(fid);
            let tr = f.map_or(0, |f| f.tracking);
            let w = lines
                .iter()
                .map(|l| f.map_or(0, |f| f.line_width(l, tr)))
                .max()
                .unwrap_or(0);
            let lh = f.map_or(0, |f| f.height()) + a.i(3, 0);
            (w, (lines.len() as i32 * lh - a.i(3, 0)).max(0))
        };
        c.api(
            lua,
            GFX,
            "graphics.getTextSize",
            Work {
                items: text.chars().count() as u64,
                ..Default::default()
            },
        );
        Ok((w, h))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::strip_markup;

    #[test]
    fn markup() {
        assert_eq!(strip_markup("*bold* and _it_"), "bold and it");
        assert_eq!(strip_markup("a**b__c"), "a*b_c");
    }
}
