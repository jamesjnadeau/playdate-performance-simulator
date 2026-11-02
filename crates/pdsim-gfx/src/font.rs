//! Playdate `.fnt` fonts.

use crate::bitmap::{Bitmap, ImageError};
use crate::types::Color;
use base64::Engine;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A glyph: its image and how far the pen moves after it.
#[derive(Debug, Clone)]
pub struct Glyph {
    pub advance: i32,
    pub bitmap: Bitmap,
}

/// A bitmap font.
#[derive(Debug, Clone)]
pub struct Font {
    pub cell_width: i32,
    pub cell_height: i32,
    pub tracking: i32,
    pub glyphs: HashMap<char, Glyph>,
    pub kerning: HashMap<(char, char), i32>,
}

const FALLBACK_FNT: &str = include_str!("../assets/fallback.fnt");
const FALLBACK_PNG: &[u8] = include_bytes!("../assets/fallback-table-8-14.png");

impl Font {
    /// The font bundled with the simulator (rasterized DejaVu Sans Mono).
    pub fn fallback() -> Font {
        let table = Bitmap::decode_png(FALLBACK_PNG).expect("bundled font image");
        Font::parse(FALLBACK_FNT, Some((table, 8, 14))).expect("bundled font")
    }

    /// The device's system font from the SDK, if it can be found.
    pub fn system_from_sdk(sdk: &Path) -> Option<Font> {
        let p = sdk.join("Resources/Fonts/Asheville/Asheville Sans 14 Bold/Asheville-Sans-14-Bold.fnt");
        Font::load(&p).ok()
    }

    /// Height of a line.
    pub fn height(&self) -> i32 {
        self.cell_height
    }

    /// Loads a `.fnt` file. `path` may omit the extension. A dependent font's
    /// image table (`<name>-table-W-H.png`) is looked up next to it.
    pub fn load(path: &Path) -> Result<Font, ImageError> {
        let fnt = if path.extension().is_some_and(|e| e == "fnt") {
            path.to_path_buf()
        } else {
            let mut p = path.as_os_str().to_owned();
            p.push(".fnt");
            PathBuf::from(p)
        };
        let text = std::fs::read_to_string(&fnt)?;
        let table = if text.lines().any(|l| l.starts_with("data=")) {
            None
        } else {
            let stem = fnt
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let dir = fnt.parent().unwrap_or(Path::new("."));
            let prefix = format!("{stem}-table-");
            let mut found = None;
            for e in std::fs::read_dir(dir)?.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if let Some(rest) = name.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".png")) {
                    let mut it = rest.split('-');
                    if let (Some(w), Some(h)) = (it.next(), it.next()) {
                        if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
                            found = Some((Bitmap::load_png(e.path())?, w, h));
                            break;
                        }
                    }
                }
            }
            Some(found.ok_or_else(|| ImageError::Other(format!("no image table for {}", fnt.display())))?)
        };
        Font::parse(&text, table)
    }

    /// Parses `.fnt` text. `table` is the image table and its cell size for
    /// dependent fonts; standalone fonts carry `data=` inline.
    pub fn parse(text: &str, table: Option<(Bitmap, i32, i32)>) -> Result<Font, ImageError> {
        let mut tracking = 0;
        let mut entries: Vec<(char, i32)> = vec![];
        let mut kerning = HashMap::new();
        let mut data_b64 = String::new();
        let (mut cw, mut ch) = (0, 0);
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with("--") {
                continue;
            }
            if let Some(v) = line.strip_prefix("tracking=") {
                tracking = v.trim().parse().unwrap_or(0);
                continue;
            }
            if let Some(v) = line.strip_prefix("data=") {
                data_b64.push_str(v.trim());
                continue;
            }
            if let Some(v) = line.strip_prefix("width=") {
                cw = v.trim().parse().unwrap_or(0);
                continue;
            }
            if let Some(v) = line.strip_prefix("height=") {
                ch = v.trim().parse().unwrap_or(0);
                continue;
            }
            if line.contains('=') && !line.contains('\t') && !line.contains(' ') {
                continue; // other metadata (datalen=, ...)
            }
            let mut parts = line.splitn(2, ['\t', ' ']);
            let tok = parts.next().unwrap_or_default();
            let val: i32 = parts.next().unwrap_or("").trim().parse().unwrap_or(0);
            if tok == "space" {
                entries.push((' ', val));
                continue;
            }
            let chars: Vec<char> = tok.chars().collect();
            match chars.len() {
                1 => entries.push((chars[0], val)),
                2 => {
                    kerning.insert((chars[0], chars[1]), val);
                }
                _ => {}
            }
        }
        let (table, cw, ch) = match table {
            Some(t) => t,
            None => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data_b64.as_bytes())
                    .map_err(|e| ImageError::Other(format!("font data: {e}")))?;
                (Bitmap::decode_png(&bytes[..])?, cw, ch)
            }
        };
        if cw <= 0 || ch <= 0 {
            return Err(ImageError::Other("font has no cell size".into()));
        }
        let cols = (table.width / cw).max(1);
        let mut glyphs = HashMap::new();
        for (i, (c, adv)) in entries.into_iter().enumerate() {
            let i = i as i32;
            let r = crate::types::Rect::new((i % cols) * cw, (i / cols) * ch, cw, ch);
            let mut bm = table.crop(r);
            if bm.mask.is_none() {
                // Fonts without alpha: white pixels are transparent.
                let mut m = Bitmap::new(cw, ch, Color::Clear);
                for y in 0..ch {
                    for x in 0..cw {
                        if !bm.get(x, y) {
                            m.set(x, y, false);
                        }
                    }
                }
                bm = m;
            }
            glyphs.insert(
                c,
                Glyph {
                    advance: adv,
                    bitmap: bm,
                },
            );
        }
        Ok(Font {
            cell_width: cw,
            cell_height: ch,
            tracking,
            glyphs,
            kerning,
        })
    }

    /// The glyph for `c`, falling back to `?`.
    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c).or_else(|| self.glyphs.get(&'?'))
    }

    /// Kerning adjustment between two characters.
    pub fn kern(&self, a: char, b: char) -> i32 {
        self.kerning.get(&(a, b)).copied().unwrap_or(0)
    }

    /// Width of a single line of text.
    pub fn line_width(&self, line: &str, tracking: i32) -> i32 {
        let chars: Vec<char> = line.chars().collect();
        let mut w = 0;
        for (i, c) in chars.iter().enumerate() {
            if let Some(g) = self.glyph(*c) {
                w += g.advance;
                if i + 1 < chars.len() {
                    w += tracking + self.kern(*c, chars[i + 1]);
                }
            }
        }
        w
    }

    /// Memory the font occupies.
    pub fn memory_bytes(&self) -> u64 {
        self.glyphs.values().map(|g| g.bitmap.memory_bytes()).sum::<u64>() + 64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_font_has_ascii() {
        let f = Font::fallback();
        assert_eq!(f.cell_height, 14);
        for c in ' '..='~' {
            assert!(f.glyphs.contains_key(&c), "missing {c:?}");
        }
        assert_eq!(f.line_width("ab", 0), 14);
        assert_eq!(f.line_width("ab", 2), 16);
        // 'A' has some ink.
        let a = &f.glyphs[&'A'].bitmap;
        assert!((0..14).any(|y| (0..8).any(|x| a.opaque(x, y))));
    }

    #[test]
    fn parses_kerning_and_space() {
        let mut table = Bitmap::new(4, 2, Color::White);
        table.set(0, 0, false);
        let f = Font::parse("tracking=2\nspace\t3\nA\t2\nAA\t-1\n", Some((table, 2, 2))).unwrap();
        assert_eq!(f.tracking, 2);
        assert_eq!(f.glyphs[&' '].advance, 3);
        assert_eq!(f.kern('A', 'A'), -1);
        assert_eq!(f.line_width("AA", 2), 2 + 2 - 1 + 2);
    }
}
