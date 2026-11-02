//! Image tables: `name-table-W-H.png` grids or `name-1.png, name-2.png, ...`
//! sequences.

use crate::bitmap::{Bitmap, ImageError};
use crate::types::Rect;
use std::path::Path;

/// Loads an image table. `path` is the table name without the
/// `-table-W-H.png` suffix (for example `images/player`).
pub fn load_table(path: &Path) -> Result<Vec<Bitmap>, ImageError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_name().and_then(|s| s.to_str()).unwrap_or_default();
    let stem = stem.strip_suffix(".png").unwrap_or(stem);
    let prefix = format!("{stem}-table-");
    let mut seq: Vec<(u32, std::path::PathBuf)> = vec![];
    for e in std::fs::read_dir(dir)?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if let Some(rest) = name.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".png")) {
            let mut it = rest.split('-');
            if let (Some(w), Some(h)) = (it.next(), it.next()) {
                if let (Ok(w), Ok(h)) = (w.parse::<i32>(), h.parse::<i32>()) {
                    return Ok(split_grid(&Bitmap::load_png(e.path())?, w, h));
                }
            }
        }
        if let Some(n) = name
            .strip_prefix(stem)
            .and_then(|r| r.strip_prefix('-'))
            .and_then(|r| r.strip_suffix(".png"))
            .and_then(|r| r.parse::<u32>().ok())
        {
            seq.push((n, e.path()));
        }
    }
    if seq.is_empty() {
        return Err(ImageError::Other(format!(
            "image table not found: {}",
            path.display()
        )));
    }
    seq.sort();
    seq.into_iter().map(|(_, p)| Bitmap::load_png(p)).collect()
}

/// Splits a grid image into cells of `w`×`h`, row-major.
pub fn split_grid(img: &Bitmap, w: i32, h: i32) -> Vec<Bitmap> {
    if w <= 0 || h <= 0 {
        return vec![];
    }
    let cols = img.width / w;
    let rows = img.height / h;
    let mut v = vec![];
    for r in 0..rows {
        for c in 0..cols {
            v.push(img.crop(Rect::new(c * w, r * h, w, h)));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Color;

    #[test]
    fn grid_split() {
        let mut img = Bitmap::new(8, 4, Color::White);
        img.set(4, 0, false);
        let cells = split_grid(&img, 4, 2);
        assert_eq!(cells.len(), 4);
        assert!(!cells[1].get(0, 0));
        assert!(cells[0].get(0, 0));
    }

    #[test]
    fn loads_grid_and_sequence_from_disk() {
        let dir = std::env::temp_dir().join(format!("pdsim-table-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = Bitmap::new(6, 3, Color::Black);
        std::fs::write(dir.join("hero-table-3-3.png"), img.encode_png().unwrap()).unwrap();
        std::fs::write(dir.join("walk-2.png"), img.encode_png().unwrap()).unwrap();
        std::fs::write(
            dir.join("walk-1.png"),
            Bitmap::new(1, 1, Color::White).encode_png().unwrap(),
        )
        .unwrap();
        assert_eq!(load_table(&dir.join("hero")).unwrap().len(), 2);
        let walk = load_table(&dir.join("walk")).unwrap();
        assert_eq!(walk.len(), 2);
        assert_eq!(walk[0].width, 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
