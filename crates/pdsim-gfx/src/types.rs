//! Colors, draw modes and other small graphics types.

/// An 8×8 pattern: 8 rows of pixel bits (MSB is the leftmost pixel, 1 is
/// white) plus 8 rows of mask bits (1 is opaque).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pattern {
    pub bits: [u8; 8],
    pub mask: [u8; 8],
}

impl Pattern {
    /// An opaque pattern.
    pub fn opaque(bits: [u8; 8]) -> Self {
        Pattern {
            bits,
            mask: [0xff; 8],
        }
    }

    /// The pixel at `(x, y)` in screen coordinates: `None` when transparent,
    /// `Some(true)` for white.
    #[inline]
    pub fn at(&self, x: i32, y: i32) -> Option<bool> {
        let row = (y & 7) as usize;
        let bit = 7 - (x & 7) as u32;
        if (self.mask[row] >> bit) & 1 == 0 {
            None
        } else {
            Some((self.bits[row] >> bit) & 1 == 1)
        }
    }

    /// A Bayer-ordered dither pattern of `alpha` (0 = black ... 1 = white),
    /// as `setDitherPattern` produces. `alpha` is the fraction of white pixels.
    pub fn dither(alpha: f32) -> Self {
        const BAYER: [[u8; 8]; 8] = [
            [0, 32, 8, 40, 2, 34, 10, 42],
            [48, 16, 56, 24, 50, 18, 58, 26],
            [12, 44, 4, 36, 14, 46, 6, 38],
            [60, 28, 52, 20, 62, 30, 54, 22],
            [3, 35, 11, 43, 1, 33, 9, 41],
            [51, 19, 59, 27, 49, 17, 57, 25],
            [15, 47, 7, 39, 13, 45, 5, 37],
            [63, 31, 55, 23, 61, 29, 53, 21],
        ];
        let level = (alpha.clamp(0.0, 1.0) * 64.0).round() as u8;
        let mut bits = [0u8; 8];
        for (y, row) in BAYER.iter().enumerate() {
            for (x, v) in row.iter().enumerate() {
                if *v < level {
                    bits[y] |= 0x80 >> x;
                }
            }
        }
        Pattern::opaque(bits)
    }
}

/// A drawing color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Black,
    White,
    /// Makes pixels transparent (only meaningful on bitmaps with a mask).
    Clear,
    /// Inverts the destination.
    Xor,
    Pattern(Pattern),
}

impl Color {
    /// From the numeric `kColor*` constants (0 black, 1 white, 2 clear, 3 xor).
    pub fn from_index(i: i64) -> Color {
        match i {
            0 => Color::Black,
            1 => Color::White,
            2 => Color::Clear,
            3 => Color::Xor,
            _ => Color::Black,
        }
    }
}

/// How bitmap pixels combine with the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DrawMode {
    #[default]
    Copy,
    WhiteTransparent,
    BlackTransparent,
    FillWhite,
    FillBlack,
    Xor,
    Nxor,
    Inverted,
}

impl DrawMode {
    /// From the numeric `kDrawMode*` constants.
    pub fn from_index(i: i64) -> DrawMode {
        use DrawMode::*;
        match i {
            1 => WhiteTransparent,
            2 => BlackTransparent,
            3 => FillWhite,
            4 => FillBlack,
            5 => Xor,
            6 => Nxor,
            7 => Inverted,
            _ => Copy,
        }
    }

    /// The numeric constant.
    pub fn index(self) -> i64 {
        self as i64
    }

    /// From the string names Lua accepts ("copy", "fillWhite", ...).
    pub fn from_name(s: &str) -> Option<DrawMode> {
        use DrawMode::*;
        Some(match s {
            "copy" => Copy,
            "inverted" => Inverted,
            "XOR" => Xor,
            "NXOR" => Nxor,
            "whiteTransparent" => WhiteTransparent,
            "blackTransparent" => BlackTransparent,
            "fillWhite" => FillWhite,
            "fillBlack" => FillBlack,
            _ => return None,
        })
    }

    /// The new destination pixel for source pixel `src` over `dst`, or
    /// `None` to leave the destination untouched.
    #[inline]
    pub fn apply(self, src: bool, dst: bool) -> Option<bool> {
        use DrawMode::*;
        match self {
            Copy => Some(src),
            WhiteTransparent => (!src).then_some(false),
            BlackTransparent => src.then_some(true),
            FillWhite => Some(true),
            FillBlack => Some(false),
            Xor => Some(dst ^ src),
            Nxor => Some(!(dst ^ src)),
            Inverted => Some(!src),
        }
    }
}

/// Bitmap flip flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Flip {
    #[default]
    Unflipped,
    FlippedX,
    FlippedY,
    FlippedXY,
}

impl Flip {
    /// From the numeric constants (0..3) or strings ("flipX", "flipY", "flipXY").
    pub fn from_index(i: i64) -> Flip {
        match i {
            1 => Flip::FlippedX,
            2 => Flip::FlippedY,
            3 => Flip::FlippedXY,
            _ => Flip::Unflipped,
        }
    }

    /// From the string names.
    pub fn from_name(s: &str) -> Flip {
        match s {
            "flipX" => Flip::FlippedX,
            "flipY" => Flip::FlippedY,
            "flipXY" => Flip::FlippedXY,
            _ => Flip::Unflipped,
        }
    }

    pub fn x(self) -> bool {
        matches!(self, Flip::FlippedX | Flip::FlippedXY)
    }

    pub fn y(self) -> bool {
        matches!(self, Flip::FlippedY | Flip::FlippedXY)
    }
}

/// Line end cap style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Square,
    Round,
}

/// Where rectangle and ellipse strokes are drawn relative to the shape edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrokeLocation {
    #[default]
    Centered,
    Inside,
    Outside,
}

/// Polygon fill rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillRule {
    NonZero,
    #[default]
    EvenOdd,
}

/// An integer rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    /// Intersection (may be empty).
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        Rect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
    }

    /// Smallest rectangle containing both.
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = (self.x + self.w).max(o.x + o.w);
        let y1 = (self.y + self.h).max(o.y + o.h);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// Text wrapping modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WrapMode {
    Clip,
    Character,
    #[default]
    Word,
}

/// Text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_modes_follow_the_docs() {
        // (src, dst) -> result, white = true
        assert_eq!(DrawMode::Copy.apply(false, true), Some(false));
        assert_eq!(DrawMode::WhiteTransparent.apply(true, false), None);
        assert_eq!(DrawMode::WhiteTransparent.apply(false, true), Some(false));
        assert_eq!(DrawMode::BlackTransparent.apply(false, true), None);
        assert_eq!(DrawMode::FillWhite.apply(false, false), Some(true));
        assert_eq!(DrawMode::FillBlack.apply(true, true), Some(false));
        // XOR on white: black drawn white, white drawn black.
        assert_eq!(DrawMode::Xor.apply(false, true), Some(true));
        assert_eq!(DrawMode::Xor.apply(true, true), Some(false));
        // NXOR on black: black drawn white, white drawn black.
        assert_eq!(DrawMode::Nxor.apply(false, false), Some(true));
        assert_eq!(DrawMode::Nxor.apply(true, false), Some(false));
        assert_eq!(DrawMode::Inverted.apply(true, true), Some(false));
    }

    #[test]
    fn dither_levels() {
        assert_eq!(Pattern::dither(0.0).bits, [0; 8]);
        assert_eq!(Pattern::dither(1.0).bits, [0xff; 8]);
        let half: u32 = Pattern::dither(0.5).bits.iter().map(|b| b.count_ones()).sum();
        assert_eq!(half, 32);
    }

    #[test]
    fn pattern_lookup_wraps() {
        let p = Pattern::opaque([0x80, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(p.at(0, 0), Some(true));
        assert_eq!(p.at(8, 8), Some(true));
        assert_eq!(p.at(1, 0), Some(false));
        assert_eq!(p.at(-8, 0), Some(true));
    }

    #[test]
    fn rect_ops() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), Rect::new(5, 5, 5, 5));
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
        assert!(a.intersect(&Rect::new(20, 20, 1, 1)).is_empty());
    }
}
