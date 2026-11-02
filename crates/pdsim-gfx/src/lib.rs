//! 1-bit graphics for the Playdate performance simulator.
//!
//! This crate draws into a 400×240 1-bit framebuffer the way the Playdate
//! does, and measures the work each call does (pixels, rows, glyphs, ...) so
//! the cost model can charge it.

pub mod bitmap;
pub mod font;
pub mod graphics;
pub mod painter;
pub mod shapes;
pub mod sprite;
pub mod table;
pub mod types;

pub use bitmap::{Bitmap, ImageError};
pub use font::Font;
pub use graphics::{BitmapId, FontId, Graphics, State, TextLayout, FRAME};
pub use sprite::{SpriteId, SpriteSystem};
pub use types::*;
