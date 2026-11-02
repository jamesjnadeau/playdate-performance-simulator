//! 1-bit bitmaps with an optional mask.

use crate::types::{Color, Rect};
use std::path::Path;

/// Errors loading images.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("png: {0}")]
    Png(#[from] png::DecodingError),
    #[error("png encode: {0}")]
    PngEncode(#[from] png::EncodingError),
    #[error("{0}")]
    Other(String),
}

/// A 1-bit image. Pixel bits are 1 for white; mask bits are 1 for opaque.
/// Rows are packed MSB first, like the device.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bitmap {
    pub width: i32,
    pub height: i32,
    pub rowbytes: usize,
    pub data: Vec<u8>,
    pub mask: Option<Vec<u8>>,
}

impl Bitmap {
    /// A new bitmap filled with `bg`. `Color::Clear` creates a masked,
    /// fully transparent bitmap.
    pub fn new(width: i32, height: i32, bg: Color) -> Bitmap {
        let width = width.max(0);
        let height = height.max(0);
        let rowbytes = (width as usize).div_ceil(8);
        Self::with_rowbytes(width, height, rowbytes, bg)
    }

    /// A new bitmap with an explicit row stride.
    pub fn with_rowbytes(width: i32, height: i32, rowbytes: usize, bg: Color) -> Bitmap {
        let n = rowbytes * height.max(0) as usize;
        let fill = match bg {
            Color::White => 0xff,
            _ => 0x00,
        };
        let mask = matches!(bg, Color::Clear).then(|| vec![0u8; n]);
        let mut b = Bitmap {
            width,
            height,
            rowbytes,
            data: vec![fill; n],
            mask,
        };
        if let Color::Pattern(p) = bg {
            for y in 0..height {
                for x in 0..width {
                    b.set(x, y, p.at(x, y).unwrap_or(false));
                }
            }
        }
        b
    }

    /// The 400×240 framebuffer layout (52 bytes per row), white.
    pub fn frame() -> Bitmap {
        Self::with_rowbytes(
            pdsim_core::LCD_WIDTH as i32,
            pdsim_core::LCD_HEIGHT as i32,
            pdsim_core::LCD_ROWSIZE,
            Color::White,
        )
    }

    /// Bounds as a rect at the origin.
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    #[inline]
    fn idx(&self, x: i32, y: i32) -> (usize, u8) {
        (y as usize * self.rowbytes + (x as usize >> 3), 0x80u8 >> (x & 7))
    }

    #[inline]
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width && y < self.height
    }

    /// Whether the pixel is white. Out of bounds reads as black.
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> bool {
        if !self.in_bounds(x, y) {
            return false;
        }
        let (i, b) = self.idx(x, y);
        self.data[i] & b != 0
    }

    /// Sets the pixel color (true = white) and makes it opaque.
    #[inline]
    pub fn set(&mut self, x: i32, y: i32, white: bool) {
        if !self.in_bounds(x, y) {
            return;
        }
        let (i, b) = self.idx(x, y);
        if white {
            self.data[i] |= b;
        } else {
            self.data[i] &= !b;
        }
        if let Some(m) = &mut self.mask {
            m[i] |= b;
        }
    }

    /// Whether the pixel is opaque.
    #[inline]
    pub fn opaque(&self, x: i32, y: i32) -> bool {
        if !self.in_bounds(x, y) {
            return false;
        }
        match &self.mask {
            None => true,
            Some(m) => {
                let (i, b) = self.idx(x, y);
                m[i] & b != 0
            }
        }
    }

    /// Makes the pixel transparent (adds a mask if needed).
    pub fn clear_pixel(&mut self, x: i32, y: i32) {
        if !self.in_bounds(x, y) {
            return;
        }
        let n = self.data.len();
        let (i, b) = self.idx(x, y);
        let m = self.mask.get_or_insert_with(|| vec![0xff; n]);
        m[i] &= !b;
    }

    /// Fills the whole bitmap.
    pub fn clear(&mut self, c: Color) {
        *self = Self::with_rowbytes(
            self.width,
            self.height,
            self.rowbytes,
            match c {
                Color::Clear => Color::Clear,
                other => other,
            },
        );
    }

    /// Bytes of memory the bitmap occupies on the device (data + mask +
    /// a small header).
    pub fn memory_bytes(&self) -> u64 {
        (self.data.len() + self.mask.as_ref().map_or(0, |m| m.len()) + 16) as u64
    }

    /// Loads a PNG. Pixels with alpha < 50% become transparent; the rest are
    /// white when their luminance is at least 50%.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Bitmap, ImageError> {
        let f = std::fs::File::open(path)?;
        Self::decode_png(std::io::BufReader::new(f))
    }

    /// Decodes PNG bytes.
    pub fn decode_png<R: std::io::Read>(r: R) -> Result<Bitmap, ImageError> {
        let mut dec = png::Decoder::new(r);
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info()?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf)?;
        let (w, h) = (info.width as i32, info.height as i32);
        let ch = match info.color_type {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            png::ColorType::Indexed => return Err(ImageError::Other("indexed png not expanded".into())),
        };
        let mut b = Bitmap::new(w, h, Color::Black);
        let mut any_transparent = false;
        let stride = info.line_size;
        let mut transparent = vec![];
        for y in 0..h {
            for x in 0..w {
                let p = &buf[y as usize * stride + x as usize * ch..][..ch];
                let (lum, a) = match ch {
                    1 => (p[0] as u32, 255),
                    2 => (p[0] as u32, p[1]),
                    3 => (
                        (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000,
                        255,
                    ),
                    _ => (
                        (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000,
                        p[3],
                    ),
                };
                b.set(x, y, lum >= 128);
                if a < 128 {
                    any_transparent = true;
                    transparent.push((x, y));
                }
            }
        }
        if any_transparent {
            for (x, y) in transparent {
                b.clear_pixel(x, y);
            }
        }
        Ok(b)
    }

    /// Encodes as a grayscale+alpha PNG (used for golden tests and
    /// screenshots).
    pub fn encode_png(&self) -> Result<Vec<u8>, ImageError> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width as u32, self.height as u32);
            enc.set_color(png::ColorType::GrayscaleAlpha);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header()?;
            let mut data = Vec::with_capacity((self.width * self.height * 2) as usize);
            for y in 0..self.height {
                for x in 0..self.width {
                    data.push(if self.get(x, y) { 255 } else { 0 });
                    data.push(if self.opaque(x, y) { 255 } else { 0 });
                }
            }
            w.write_image_data(&data)?;
        }
        Ok(out)
    }

    /// A copy of a sub-rectangle.
    pub fn crop(&self, r: Rect) -> Bitmap {
        let mut b = Bitmap::new(
            r.w,
            r.h,
            if self.mask.is_some() {
                Color::Clear
            } else {
                Color::Black
            },
        );
        for y in 0..r.h {
            for x in 0..r.w {
                let (sx, sy) = (r.x + x, r.y + y);
                if self.opaque(sx, sy) {
                    b.set(x, y, self.get(sx, sy));
                }
            }
        }
        b
    }

    /// The frame as bits (1 = white), one bit per pixel, rows of 52 bytes.
    /// Only valid for the framebuffer layout.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_and_mask() {
        let mut b = Bitmap::new(10, 3, Color::Clear);
        assert!(!b.opaque(3, 1));
        b.set(3, 1, true);
        assert!(b.opaque(3, 1));
        assert!(b.get(3, 1));
        b.set(3, 1, false);
        assert!(!b.get(3, 1));
        b.clear_pixel(3, 1);
        assert!(!b.opaque(3, 1));
        assert_eq!(b.rowbytes, 2);
    }

    #[test]
    fn frame_layout() {
        let f = Bitmap::frame();
        assert_eq!(f.rowbytes, 52);
        assert_eq!(f.data.len(), 52 * 240);
        assert!(f.get(399, 239));
    }

    #[test]
    fn png_round_trip() {
        let mut b = Bitmap::new(9, 4, Color::Clear);
        b.set(0, 0, true);
        b.set(8, 3, false);
        let png = b.encode_png().unwrap();
        let back = Bitmap::decode_png(&png[..]).unwrap();
        assert_eq!(back.width, 9);
        assert!(back.get(0, 0) && back.opaque(0, 0));
        assert!(!back.get(8, 3) && back.opaque(8, 3));
        assert!(!back.opaque(4, 2));
    }
}
