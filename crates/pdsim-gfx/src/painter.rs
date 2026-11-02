//! Low-level pixel writer used by every primitive. It applies the draw
//! offset, clip rect and stencil, and counts the work done.

use crate::bitmap::Bitmap;
use crate::types::{Color, DrawMode, Rect};

/// Writes pixels into one target bitmap.
pub struct Painter<'a> {
    pub(crate) dst: &'a mut Bitmap,
    pub(crate) stencil: Option<(&'a Bitmap, bool)>,
    /// Clip in target coordinates.
    pub(crate) clip: Rect,
    pub(crate) ox: i32,
    pub(crate) oy: i32,
    pub(crate) pixels: u64,
    pub(crate) rows: Vec<bool>,
}

impl<'a> Painter<'a> {
    pub(crate) fn new(
        dst: &'a mut Bitmap,
        stencil: Option<(&'a Bitmap, bool)>,
        clip: Option<Rect>,
        offset: (i32, i32),
    ) -> Self {
        let b = dst.bounds();
        let clip = clip.map_or(b, |c| c.intersect(&b));
        let h = dst.height.max(0) as usize;
        Painter {
            dst,
            stencil,
            clip,
            ox: offset.0,
            oy: offset.1,
            pixels: 0,
            rows: vec![false; h],
        }
    }

    /// Number of distinct target rows touched.
    pub fn rows_touched(&self) -> u64 {
        self.rows.iter().filter(|r| **r).count() as u64
    }

    #[inline]
    fn visible(&self, tx: i32, ty: i32) -> bool {
        if !self.clip.contains(tx, ty) {
            return false;
        }
        if let Some((s, tile)) = self.stencil {
            let (sx, sy) = if tile && s.width > 0 && s.height > 0 {
                (tx.rem_euclid(s.width), ty.rem_euclid(s.height))
            } else {
                (tx, ty)
            };
            if !(s.get(sx, sy) && s.opaque(sx, sy)) {
                return false;
            }
        }
        true
    }

    #[inline]
    fn touch(&mut self, ty: i32) {
        self.pixels += 1;
        self.rows[ty as usize] = true;
    }

    /// Plots a pixel in user coordinates with a shape color.
    #[inline]
    pub fn plot(&mut self, x: i32, y: i32, c: Color) {
        let (tx, ty) = (x + self.ox, y + self.oy);
        self.plot_target(tx, ty, c);
    }

    /// Plots a pixel in target coordinates with a shape color.
    #[inline]
    pub fn plot_target(&mut self, tx: i32, ty: i32, c: Color) {
        if !self.visible(tx, ty) {
            return;
        }
        match c {
            Color::Black => self.dst.set(tx, ty, false),
            Color::White => self.dst.set(tx, ty, true),
            Color::Clear => self.dst.clear_pixel(tx, ty),
            Color::Xor => {
                let v = self.dst.get(tx, ty);
                self.dst.set(tx, ty, !v)
            }
            Color::Pattern(p) => match p.at(tx, ty) {
                Some(w) => self.dst.set(tx, ty, w),
                None => return,
            },
        }
        self.touch(ty);
    }

    /// Plots a source image pixel through a draw mode (target coordinates).
    #[inline]
    pub fn blit(&mut self, tx: i32, ty: i32, src_white: bool, mode: DrawMode) {
        if !self.visible(tx, ty) {
            return;
        }
        let dst = self.dst.get(tx, ty);
        if let Some(v) = mode.apply(src_white, dst) {
            self.dst.set(tx, ty, v);
            self.touch(ty);
        }
    }

    /// Fills the inclusive horizontal span `x0..=x1` on row `y` (user coords).
    pub fn hspan(&mut self, x0: i32, x1: i32, y: i32, c: Color) {
        let (a, b) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        let ty = y + self.oy;
        if ty < self.clip.y || ty >= self.clip.y + self.clip.h {
            return;
        }
        let a = (a + self.ox).max(self.clip.x);
        let b = (b + self.ox).min(self.clip.x + self.clip.w - 1);
        for tx in a..=b {
            self.plot_target(tx, ty, c);
        }
    }

    /// Fills a rectangle (user coords).
    pub fn fill_rect(&mut self, r: Rect, c: Color) {
        if r.is_empty() {
            return;
        }
        for y in r.y..r.y + r.h {
            self.hspan(r.x, r.x + r.w - 1, y, c);
        }
    }

    /// Target-space clip.
    pub fn clip(&self) -> Rect {
        self.clip
    }

    /// Draw offset.
    pub fn offset(&self) -> (i32, i32) {
        (self.ox, self.oy)
    }
}
