//! The graphics context: resources, drawing state and every drawing call.
//!
//! Each drawing call returns the [`Work`] it did (pixels and rows touched,
//! glyphs drawn, ...). The caller turns that into device cycles with the
//! calibration's API cost table.

use crate::bitmap::{Bitmap, ImageError};
use crate::font::Font;
use crate::painter::Painter;
use crate::shapes;
use crate::types::*;
use pdsim_core::{Work, LCD_HEIGHT};
use std::path::Path;

/// Handle to a bitmap owned by [`Graphics`].
pub type BitmapId = u32;
/// Handle to a font owned by [`Graphics`].
pub type FontId = u32;
/// The screen framebuffer.
pub const FRAME: BitmapId = 0;

/// Drawing state, saved and restored by push/popContext.
#[derive(Debug, Clone)]
pub struct State {
    pub target: BitmapId,
    pub draw_offset: (i32, i32),
    /// Clip rect in target coordinates.
    pub clip: Option<Rect>,
    pub color: Color,
    pub draw_mode: DrawMode,
    pub line_width: i32,
    pub line_cap: LineCap,
    pub stroke: StrokeLocation,
    pub stencil: Option<(BitmapId, bool)>,
    pub font: Option<FontId>,
    pub tracking: Option<i32>,
    pub leading: i32,
    pub polygon_rule: FillRule,
}

impl Default for State {
    fn default() -> Self {
        State {
            target: FRAME,
            draw_offset: (0, 0),
            clip: None,
            color: Color::Black,
            draw_mode: DrawMode::Copy,
            line_width: 1,
            line_cap: LineCap::Butt,
            stroke: StrokeLocation::Centered,
            stencil: None,
            font: None,
            tracking: None,
            leading: 0,
            polygon_rule: FillRule::EvenOdd,
        }
    }
}

/// Graphics resources and state.
pub struct Graphics {
    bitmaps: Vec<Option<Bitmap>>,
    free_bitmaps: Vec<BitmapId>,
    fonts: Vec<Option<Font>>,
    pub state: State,
    stack: Vec<State>,
    dirty_rows: [bool; LCD_HEIGHT],
    display: Bitmap,
    system_font: FontId,
    pub background_color: Color,
    image_bytes: u64,
    /// Whether the display is mirrored/inverted (display.setInverted).
    pub inverted: bool,
    /// Display scale (display.setScale), 1, 2, 4 or 8.
    pub scale: i32,
    /// Display offset (display.setOffset).
    pub display_offset: (i32, i32),
}

impl Default for Graphics {
    fn default() -> Self {
        Self::new(None)
    }
}

/// Result of laying out text: size and whether it was cut short.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextLayout {
    pub width: i32,
    pub height: i32,
    pub truncated: bool,
}

impl Graphics {
    /// A new context with a white framebuffer. With an SDK path the device
    /// system font is used, otherwise the bundled fallback font.
    pub fn new(sdk: Option<&Path>) -> Self {
        let font = sdk.and_then(Font::system_from_sdk).unwrap_or_else(Font::fallback);
        let mut g = Graphics {
            bitmaps: vec![Some(Bitmap::frame())],
            free_bitmaps: vec![],
            fonts: vec![],
            state: State::default(),
            stack: vec![],
            dirty_rows: [false; LCD_HEIGHT],
            display: Bitmap::frame(),
            system_font: 0,
            background_color: Color::White,
            image_bytes: 0,
            inverted: false,
            scale: 1,
            display_offset: (0, 0),
        };
        g.system_font = g.add_font(font);
        g
    }

    // ---------------------------------------------------------------
    // Resources

    /// Stores a bitmap and returns its handle.
    pub fn add_bitmap(&mut self, b: Bitmap) -> BitmapId {
        self.image_bytes += b.memory_bytes();
        if let Some(id) = self.free_bitmaps.pop() {
            self.bitmaps[id as usize] = Some(b);
            id
        } else {
            self.bitmaps.push(Some(b));
            (self.bitmaps.len() - 1) as BitmapId
        }
    }

    /// Frees a bitmap. The framebuffer cannot be freed.
    pub fn free_bitmap(&mut self, id: BitmapId) {
        if id == FRAME {
            return;
        }
        if let Some(slot) = self.bitmaps.get_mut(id as usize) {
            if let Some(b) = slot.take() {
                self.image_bytes = self.image_bytes.saturating_sub(b.memory_bytes());
                self.free_bitmaps.push(id);
            }
        }
    }

    /// A bitmap by handle.
    pub fn bitmap(&self, id: BitmapId) -> Option<&Bitmap> {
        self.bitmaps.get(id as usize).and_then(|b| b.as_ref())
    }

    /// A mutable bitmap by handle.
    pub fn bitmap_mut(&mut self, id: BitmapId) -> Option<&mut Bitmap> {
        self.bitmaps.get_mut(id as usize).and_then(|b| b.as_mut())
    }

    /// Creates a bitmap filled with `bg`.
    pub fn new_bitmap(&mut self, w: i32, h: i32, bg: Color) -> BitmapId {
        self.add_bitmap(Bitmap::new(w, h, bg))
    }

    /// Loads a PNG image.
    pub fn load_bitmap(&mut self, path: &Path) -> Result<BitmapId, ImageError> {
        Ok(self.add_bitmap(Bitmap::load_png(path)?))
    }

    /// Copies a bitmap.
    pub fn copy_bitmap(&mut self, id: BitmapId) -> Option<BitmapId> {
        let b = self.bitmap(id)?.clone();
        Some(self.add_bitmap(b))
    }

    /// Stores a font.
    pub fn add_font(&mut self, f: Font) -> FontId {
        self.image_bytes += f.memory_bytes();
        self.fonts.push(Some(f));
        (self.fonts.len() - 1) as FontId
    }

    /// Loads a font file.
    pub fn load_font(&mut self, path: &Path) -> Result<FontId, ImageError> {
        Ok(self.add_font(Font::load(path)?))
    }

    /// A font by handle.
    pub fn font(&self, id: FontId) -> Option<&Font> {
        self.fonts.get(id as usize).and_then(|f| f.as_ref())
    }

    /// Sets a font's tracking.
    pub fn set_font_tracking(&mut self, id: FontId, tracking: i32) {
        if let Some(Some(f)) = self.fonts.get_mut(id as usize) {
            f.tracking = tracking;
        }
    }

    /// The system font.
    pub fn system_font(&self) -> FontId {
        self.system_font
    }

    /// The current font.
    pub fn current_font(&self) -> FontId {
        self.state.font.unwrap_or(self.system_font)
    }

    /// Bytes of memory used by bitmaps and fonts (excluding the frame).
    pub fn image_memory(&self) -> u64 {
        self.image_bytes
    }

    // ---------------------------------------------------------------
    // State

    /// Saves the state and makes `target` the drawing target (or the frame).
    pub fn push_context(&mut self, target: Option<BitmapId>) {
        self.stack.push(self.state.clone());
        let t = target.unwrap_or(FRAME);
        self.state = State {
            target: t,
            font: self.state.font,
            ..State::default()
        };
    }

    /// Restores the previous state.
    pub fn pop_context(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.state = s;
        }
    }

    /// Context stack depth.
    pub fn context_depth(&self) -> usize {
        self.stack.len()
    }

    /// Sets the clip rect in user (draw-offset) coordinates.
    pub fn set_clip_rect(&mut self, r: Rect) {
        let (ox, oy) = self.state.draw_offset;
        self.state.clip = Some(Rect::new(r.x + ox, r.y + oy, r.w, r.h));
    }

    /// Sets the clip rect in screen coordinates.
    pub fn set_screen_clip_rect(&mut self, r: Rect) {
        self.state.clip = Some(r);
    }

    /// Clears the clip rect.
    pub fn clear_clip_rect(&mut self) {
        self.state.clip = None;
    }

    /// The clip rect in user coordinates (the full target when unset).
    pub fn clip_rect(&self) -> Rect {
        let (ox, oy) = self.state.draw_offset;
        let c = self.state.clip.unwrap_or_else(|| {
            self.bitmap(self.state.target)
                .map(|b| b.bounds())
                .unwrap_or_default()
        });
        Rect::new(c.x - ox, c.y - oy, c.w, c.h)
    }

    // ---------------------------------------------------------------
    // Painting

    fn paint<F: FnOnce(&mut Painter)>(&mut self, f: F) -> Work {
        let tid = self.state.target;
        let Some(mut target) = self.bitmaps.get_mut(tid as usize).and_then(|b| b.take()) else {
            return Work::default();
        };
        let stencil = self.state.stencil.and_then(|(sid, tile)| {
            self.bitmaps
                .get(sid as usize)
                .and_then(|b| b.as_ref())
                .map(|b| (b, tile))
        });
        let mut p = Painter::new(&mut target, stencil, self.state.clip, self.state.draw_offset);
        f(&mut p);
        let w = Work::pixels(p.pixels, p.rows_touched());
        if tid == FRAME {
            for (i, r) in p.rows.iter().enumerate() {
                if *r {
                    self.dirty_rows[i] = true;
                }
            }
        }
        drop(p);
        self.bitmaps[tid as usize] = Some(target);
        w
    }

    /// Fills the whole target with `c`, ignoring clip and offset.
    pub fn clear(&mut self, c: Color) -> Work {
        let tid = self.state.target;
        let Some(b) = self.bitmap_mut(tid) else {
            return Work::default();
        };
        let (w, h) = (b.width, b.height);
        match c {
            Color::Black | Color::White | Color::Clear => {
                let had_mask = b.mask.is_some();
                b.clear(c);
                if had_mask && b.mask.is_none() {
                    let n = b.data.len();
                    b.mask = Some(vec![0xff; n]);
                }
            }
            _ => {
                let saved = (self.state.clip, self.state.draw_offset, self.state.stencil);
                self.state.clip = None;
                self.state.draw_offset = (0, 0);
                self.state.stencil = None;
                self.paint(|p| p.fill_rect(Rect::new(0, 0, w, h), c));
                (self.state.clip, self.state.draw_offset, self.state.stencil) = saved;
            }
        }
        if tid == FRAME {
            self.dirty_rows = [true; LCD_HEIGHT];
        }
        Work::pixels((w * h) as u64, h as u64)
    }

    /// Sets one pixel.
    pub fn draw_pixel(&mut self, x: i32, y: i32, c: Color) -> Work {
        self.paint(|p| p.plot(x, y, c))
    }

    /// Fills a rectangle with color `c`.
    pub fn fill_rect(&mut self, r: Rect, c: Color) -> Work {
        let r = normalize(r);
        self.paint(|p| p.fill_rect(r, c))
    }

    /// Strokes a rectangle with the current line width and stroke location.
    pub fn draw_rect(&mut self, r: Rect, c: Color) -> Work {
        let r = normalize(r);
        let lw = self.state.line_width.max(1);
        let outer = match self.state.stroke {
            StrokeLocation::Inside => r,
            StrokeLocation::Centered => {
                let e = lw / 2;
                Rect::new(r.x - e, r.y - e, r.w + 2 * e, r.h + 2 * e)
            }
            StrokeLocation::Outside => Rect::new(r.x - lw, r.y - lw, r.w + 2 * lw, r.h + 2 * lw),
        };
        self.paint(|p| stroke_rect(p, outer, lw, c))
    }

    /// Draws a line with the current width and cap style.
    pub fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Color) -> Work {
        let (lw, cap) = (self.state.line_width, self.state.line_cap);
        self.paint(|p| shapes::line(p, x0, y0, x1, y1, lw, cap, c))
    }

    /// Fills a triangle.
    pub fn fill_triangle(&mut self, pts: [(i32, i32); 3], c: Color) -> Work {
        let mut w = self.paint(|p| shapes::fill_polygon(p, &pts, FillRule::NonZero, c));
        w.items = 3;
        w
    }

    /// Fills a polygon.
    pub fn fill_polygon(&mut self, pts: &[(i32, i32)], rule: FillRule, c: Color) -> Work {
        let mut w = self.paint(|p| shapes::fill_polygon(p, pts, rule, c));
        w.items = pts.len() as u64;
        w
    }

    /// Strokes a polygon (closed when `closed`).
    pub fn draw_polygon(&mut self, pts: &[(i32, i32)], closed: bool, c: Color) -> Work {
        let (lw, cap) = (self.state.line_width, self.state.line_cap);
        let n = pts.len();
        let mut w = self.paint(|p| {
            let segs = if closed { n } else { n.saturating_sub(1) };
            for i in 0..segs {
                let a = pts[i];
                let b = pts[(i + 1) % n];
                shapes::line(p, a.0, a.1, b.0, b.1, lw, cap, c);
            }
        });
        w.items = n as u64;
        w
    }

    /// Fills the ellipse in `r` (an arc when the angles differ).
    pub fn fill_ellipse(&mut self, r: Rect, start: f32, end: f32, c: Color) -> Work {
        let r = normalize(r);
        self.paint(|p| shapes::fill_ellipse(p, r, start, end, c))
    }

    /// Strokes the ellipse in `r`.
    pub fn draw_ellipse(&mut self, r: Rect, lw: i32, start: f32, end: f32, c: Color) -> Work {
        let r = normalize(r);
        self.paint(|p| shapes::draw_ellipse(p, r, lw, start, end, c))
    }

    /// Fills a rounded rectangle.
    pub fn fill_round_rect(&mut self, r: Rect, radius: i32, c: Color) -> Work {
        let r = normalize(r);
        self.paint(|p| shapes::fill_round_rect(p, r, radius, c))
    }

    /// Strokes a rounded rectangle.
    pub fn draw_round_rect(&mut self, r: Rect, radius: i32, lw: i32, c: Color) -> Work {
        let r = normalize(r);
        self.paint(|p| shapes::draw_round_rect(p, r, radius, lw, c))
    }

    // ---------------------------------------------------------------
    // Bitmaps

    fn source(&self, id: BitmapId) -> Option<Bitmap> {
        self.bitmap(id).cloned()
    }

    /// Draws a bitmap with its top-left at `(x, y)` using the draw mode.
    pub fn draw_bitmap(&mut self, id: BitmapId, x: i32, y: i32, flip: Flip) -> Work {
        let Some(src) = self.source(id) else {
            return Work::default();
        };
        let mode = self.state.draw_mode;
        let mut w = self.paint(|p| blit(p, &src, x, y, flip, mode));
        w.bytes = (src.rowbytes * src.height.max(0) as usize) as u64;
        w
    }

    /// Draws a bitmap scaled by `(sx, sy)`. Negative scales flip.
    pub fn draw_scaled_bitmap(&mut self, id: BitmapId, x: i32, y: i32, sx: f32, sy: f32) -> Work {
        let Some(src) = self.source(id) else {
            return Work::default();
        };
        if sx == 0.0 || sy == 0.0 {
            return Work::default();
        }
        let mode = self.state.draw_mode;
        let dw = (src.width as f32 * sx.abs()).round() as i32;
        let dh = (src.height as f32 * sy.abs()).round() as i32;
        self.paint(|p| {
            let (ox, oy) = p.offset();
            for dy in 0..dh {
                let mut syi = (dy as f32 / sy.abs()) as i32;
                if sy < 0.0 {
                    syi = src.height - 1 - syi;
                }
                for dx in 0..dw {
                    let mut sxi = (dx as f32 / sx.abs()) as i32;
                    if sx < 0.0 {
                        sxi = src.width - 1 - sxi;
                    }
                    if src.opaque(sxi, syi) {
                        p.blit(x + dx + ox, y + dy + oy, src.get(sxi, syi), mode);
                    }
                }
            }
        })
    }

    /// Draws a bitmap rotated by `deg` degrees clockwise around the point
    /// `(x, y)`, which corresponds to the relative point `(cx, cy)` (0..1) of
    /// the scaled image.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_rotated_bitmap(
        &mut self,
        id: BitmapId,
        x: i32,
        y: i32,
        deg: f32,
        cx: f32,
        cy: f32,
        sx: f32,
        sy: f32,
    ) -> Work {
        let Some(src) = self.source(id) else {
            return Work::default();
        };
        if sx == 0.0 || sy == 0.0 {
            return Work::default();
        }
        let mode = self.state.draw_mode;
        let (sw, sh) = (src.width as f32 * sx, src.height as f32 * sy);
        let (s, c) = deg.to_radians().sin_cos();
        // Corners of the scaled image relative to the anchor.
        let ax = cx * sw;
        let ay = cy * sh;
        let corners = [(-ax, -ay), (sw - ax, -ay), (-ax, sh - ay), (sw - ax, sh - ay)];
        let rot: Vec<(f32, f32)> = corners
            .iter()
            .map(|&(u, v)| (u * c - v * s, u * s + v * c))
            .collect();
        let minx = rot.iter().map(|p| p.0).fold(f32::INFINITY, f32::min).floor() as i32;
        let maxx = rot.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max).ceil() as i32;
        let miny = rot.iter().map(|p| p.1).fold(f32::INFINITY, f32::min).floor() as i32;
        let maxy = rot.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max).ceil() as i32;
        self.paint(|p| {
            let (ox, oy) = p.offset();
            for dy in miny..maxy {
                for dx in minx..maxx {
                    let (fx, fy) = (dx as f32 + 0.5, dy as f32 + 0.5);
                    // Inverse rotate, then unscale.
                    let u = fx * c + fy * s + ax;
                    let v = -fx * s + fy * c + ay;
                    let sxi = (u / sx).floor() as i32;
                    let syi = (v / sy).floor() as i32;
                    if src.opaque(sxi, syi) {
                        p.blit(x + dx + ox, y + dy + oy, src.get(sxi, syi), mode);
                    }
                }
            }
        })
    }

    /// Tiles a bitmap over a rectangle.
    pub fn tile_bitmap(&mut self, id: BitmapId, r: Rect, flip: Flip) -> Work {
        let Some(src) = self.source(id) else {
            return Work::default();
        };
        if src.width <= 0 || src.height <= 0 {
            return Work::default();
        }
        let r = normalize(r);
        let mode = self.state.draw_mode;
        self.paint(|p| {
            let (ox, oy) = p.offset();
            for dy in 0..r.h {
                for dx in 0..r.w {
                    let mut sx = dx % src.width;
                    let mut sy = dy % src.height;
                    if flip.x() {
                        sx = src.width - 1 - sx;
                    }
                    if flip.y() {
                        sy = src.height - 1 - sy;
                    }
                    if src.opaque(sx, sy) {
                        p.blit(r.x + dx + ox, r.y + dy + oy, src.get(sx, sy), mode);
                    }
                }
            }
        })
    }

    /// Returns a new bitmap: `id` rotated by `deg` and scaled.
    pub fn rotated_bitmap(&mut self, id: BitmapId, deg: f32, sx: f32, sy: f32) -> Option<BitmapId> {
        let src = self.bitmap(id)?;
        let (sw, sh) = (src.width as f32 * sx.abs(), src.height as f32 * sy.abs());
        let (s, c) = deg.to_radians().sin_cos();
        let nw = (sw * c.abs() + sh * s.abs()).ceil() as i32;
        let nh = (sw * s.abs() + sh * c.abs()).ceil() as i32;
        let out = self.new_bitmap(nw, nh, Color::Clear);
        self.push_context(Some(out));
        self.draw_rotated_bitmap(id, nw / 2, nh / 2, deg, 0.5, 0.5, sx, sy);
        self.pop_context();
        Some(out)
    }

    /// Returns a new bitmap: `id` scaled.
    pub fn scaled_bitmap(&mut self, id: BitmapId, sx: f32, sy: f32) -> Option<BitmapId> {
        let src = self.bitmap(id)?;
        let nw = (src.width as f32 * sx.abs()).round() as i32;
        let nh = (src.height as f32 * sy.abs()).round() as i32;
        let out = self.new_bitmap(nw, nh, Color::Clear);
        self.push_context(Some(out));
        self.draw_scaled_bitmap(id, 0, 0, sx, sy);
        self.pop_context();
        Some(out)
    }

    // ---------------------------------------------------------------
    // Text

    fn tracking_for(&self, font: &Font) -> i32 {
        self.state.tracking.unwrap_or(font.tracking)
    }

    /// Size of `text` (multi-line) in `font`.
    pub fn text_size(&self, font: FontId, text: &str) -> (i32, i32) {
        let Some(f) = self.font(font) else { return (0, 0) };
        let tr = self.tracking_for(f);
        let lines: Vec<&str> = text.split('\n').collect();
        let w = lines.iter().map(|l| f.line_width(l, tr)).max().unwrap_or(0);
        let h = lines.len() as i32 * f.height() + (lines.len() as i32 - 1).max(0) * self.state.leading;
        (w, h)
    }

    /// Draws one line of text; returns pen advance and glyphs drawn.
    fn draw_line_text(&mut self, f: &Font, line: &str, x: i32, y: i32) -> (i32, Work) {
        let tr = self.tracking_for(f);
        let mode = self.state.draw_mode;
        let chars: Vec<char> = line.chars().collect();
        let mut pen = x;
        let mut w = self.paint(|p| {
            let (ox, oy) = p.offset();
            for (i, ch) in chars.iter().enumerate() {
                let Some(g) = f.glyph(*ch) else { continue };
                let b = &g.bitmap;
                for gy in 0..b.height {
                    for gx in 0..b.width {
                        if b.opaque(gx, gy) {
                            p.blit(pen + gx + ox, y + gy + oy, b.get(gx, gy), mode);
                        }
                    }
                }
                pen += g.advance;
                if i + 1 < chars.len() {
                    pen += tr + f.kern(*ch, chars[i + 1]);
                }
            }
        });
        w.items = chars.len() as u64;
        (pen - x, w)
    }

    /// Draws text (which may contain newlines) at `(x, y)` with `font`.
    /// Returns the drawn size and the work done.
    pub fn draw_text(&mut self, font: FontId, text: &str, x: i32, y: i32) -> (i32, i32, Work) {
        let Some(f) = self.font(font).cloned() else {
            return (0, 0, Work::default());
        };
        let mut work = Work::default();
        let mut maxw = 0;
        let mut ly = y;
        let mut lines = 0;
        for line in text.split('\n') {
            let (adv, w) = self.draw_line_text(&f, line, x, ly);
            work.add(w);
            maxw = maxw.max(adv);
            ly += f.height() + self.state.leading;
            lines += 1;
        }
        let h = lines * f.height() + (lines - 1).max(0) * self.state.leading;
        (maxw, h, work)
    }

    /// Breaks text into lines that fit `width`.
    pub fn wrap_text(&self, font: FontId, text: &str, width: i32, wrap: WrapMode) -> Vec<String> {
        let Some(f) = self.font(font) else { return vec![] };
        let tr = self.tracking_for(f);
        let mut out = vec![];
        for para in text.split('\n') {
            match wrap {
                WrapMode::Clip => out.push(para.to_string()),
                WrapMode::Character => {
                    let mut cur = String::new();
                    for ch in para.chars() {
                        let mut t = cur.clone();
                        t.push(ch);
                        if !cur.is_empty() && f.line_width(&t, tr) > width {
                            out.push(std::mem::take(&mut cur));
                            cur.push(ch);
                        } else {
                            cur = t;
                        }
                    }
                    out.push(cur);
                }
                WrapMode::Word => {
                    let mut cur = String::new();
                    for word in para.split(' ') {
                        let cand = if cur.is_empty() {
                            word.to_string()
                        } else {
                            format!("{cur} {word}")
                        };
                        if !cur.is_empty() && f.line_width(&cand, tr) > width {
                            out.push(std::mem::replace(&mut cur, word.to_string()));
                        } else {
                            cur = cand;
                        }
                    }
                    out.push(cur);
                }
            }
        }
        out
    }

    /// Draws text wrapped and aligned inside `r`.
    pub fn draw_text_in_rect(
        &mut self,
        font: FontId,
        text: &str,
        r: Rect,
        wrap: WrapMode,
        align: TextAlign,
    ) -> (TextLayout, Work) {
        let Some(f) = self.font(font).cloned() else {
            return (TextLayout::default(), Work::default());
        };
        let tr = self.tracking_for(&f);
        let lines = self.wrap_text(font, text, r.w, wrap);
        let saved = self.state.clip;
        let (ox, oy) = self.state.draw_offset;
        let rc = Rect::new(r.x + ox, r.y + oy, r.w, r.h);
        self.state.clip = Some(saved.map_or(rc, |c| c.intersect(&rc)));
        let mut work = Work::default();
        let mut y = r.y;
        let mut maxw = 0;
        let mut truncated = false;
        let lh = f.height() + self.state.leading;
        let mut drawn = 0;
        for line in &lines {
            if y + f.height() > r.y + r.h && drawn > 0 {
                truncated = true;
                break;
            }
            let lw = f.line_width(line, tr);
            let x = match align {
                TextAlign::Left => r.x,
                TextAlign::Center => r.x + (r.w - lw) / 2,
                TextAlign::Right => r.x + r.w - lw,
            };
            let (_, w) = self.draw_line_text(&f, line, x, y);
            work.add(w);
            maxw = maxw.max(lw.min(r.w));
            y += lh;
            drawn += 1;
        }
        self.state.clip = saved;
        let h = (drawn * lh - self.state.leading).max(0);
        (
            TextLayout {
                width: maxw,
                height: h,
                truncated,
            },
            work,
        )
    }

    // ---------------------------------------------------------------
    // Display

    /// Marks framebuffer rows `start..=end` as changed.
    pub fn mark_updated_rows(&mut self, start: i32, end: i32) {
        let s = start.clamp(0, LCD_HEIGHT as i32 - 1) as usize;
        let e = end.clamp(0, LCD_HEIGHT as i32 - 1) as usize;
        for r in s..=e {
            self.dirty_rows[r] = true;
        }
    }

    /// Rows currently marked changed.
    pub fn dirty_row_count(&self) -> u32 {
        self.dirty_rows.iter().filter(|r| **r).count() as u32
    }

    /// The dirty-row flags.
    pub fn dirty_rows(&self) -> &[bool; LCD_HEIGHT] {
        &self.dirty_rows
    }

    /// Sends the changed rows to the display and returns the row flags that
    /// were sent.
    pub fn flush_display(&mut self) -> [bool; LCD_HEIGHT] {
        let rows = self.dirty_rows;
        let frame = self.bitmaps[FRAME as usize].as_ref().expect("frame exists");
        for (y, d) in rows.iter().enumerate() {
            if *d {
                let o = y * frame.rowbytes;
                self.display.data[o..o + frame.rowbytes].copy_from_slice(&frame.data[o..o + frame.rowbytes]);
            }
        }
        self.dirty_rows = [false; LCD_HEIGHT];
        rows
    }

    /// The working framebuffer.
    pub fn frame(&self) -> &Bitmap {
        self.bitmaps[FRAME as usize].as_ref().expect("frame exists")
    }

    /// Mutable access to the working framebuffer (for raw `getFrame`
    /// writes; call [`Graphics::mark_updated_rows`] afterwards).
    pub fn frame_mut(&mut self) -> &mut Bitmap {
        self.bitmaps[FRAME as usize].as_mut().expect("frame exists")
    }

    /// What the display currently shows.
    pub fn display(&self) -> &Bitmap {
        &self.display
    }
}

fn normalize(r: Rect) -> Rect {
    let mut r = r;
    if r.w < 0 {
        r.x += r.w;
        r.w = -r.w;
    }
    if r.h < 0 {
        r.y += r.h;
        r.h = -r.h;
    }
    r
}

fn stroke_rect(p: &mut Painter, o: Rect, lw: i32, c: Color) {
    if o.is_empty() {
        return;
    }
    if lw * 2 >= o.w || lw * 2 >= o.h {
        return p.fill_rect(o, c);
    }
    p.fill_rect(Rect::new(o.x, o.y, o.w, lw), c);
    p.fill_rect(Rect::new(o.x, o.y + o.h - lw, o.w, lw), c);
    p.fill_rect(Rect::new(o.x, o.y + lw, lw, o.h - 2 * lw), c);
    p.fill_rect(Rect::new(o.x + o.w - lw, o.y + lw, lw, o.h - 2 * lw), c);
}

fn blit(p: &mut Painter, src: &Bitmap, x: i32, y: i32, flip: Flip, mode: DrawMode) {
    let (ox, oy) = p.offset();
    // Only visit the part of the source that lands inside the clip.
    let clip = p.clip();
    let tx0 = x + ox;
    let ty0 = y + oy;
    let vis = Rect::new(tx0, ty0, src.width, src.height).intersect(&clip);
    for ty in vis.y..vis.y + vis.h {
        let mut sy = ty - ty0;
        if flip.y() {
            sy = src.height - 1 - sy;
        }
        for tx in vis.x..vis.x + vis.w {
            let mut sx = tx - tx0;
            if flip.x() {
                sx = src.width - 1 - sx;
            }
            if src.opaque(sx, sy) {
                p.blit(tx, ty, src.get(sx, sy), mode);
            }
        }
    }
}
