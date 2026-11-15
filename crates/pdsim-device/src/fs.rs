//! The game's file system: the read-only game folder and a writable data
//! folder (where `playdate.datastore` and `playdate.file` writes go).

use pdsim_gfx::{Bitmap, ImageError};
use std::path::{Component, Path, PathBuf};

/// Sandboxed file access.
#[derive(Debug, Clone)]
pub struct FileSystem {
    /// The game's source folder (read-only).
    pub game_dir: PathBuf,
    /// The writable data folder.
    pub data_dir: PathBuf,
    /// Extra read-only folders for paths with a given prefix (for example
    /// `CoreLibs/` assets from the SDK).
    pub extra_roots: Vec<(String, PathBuf)>,
}

impl FileSystem {
    pub fn new(game_dir: PathBuf, data_dir: PathBuf) -> Self {
        FileSystem {
            game_dir,
            data_dir,
            extra_roots: vec![],
        }
    }

    /// Normalizes a game path, rejecting anything that escapes the sandbox.
    pub fn clean(path: &str) -> Option<PathBuf> {
        let mut out = PathBuf::new();
        for c in Path::new(path.trim_start_matches('/')).components() {
            match c {
                Component::Normal(p) => out.push(p),
                Component::CurDir => {}
                _ => return None,
            }
        }
        Some(out)
    }

    /// Every place `path` may be read from, in lookup order: the data
    /// folder, the game folder, then any extra root whose prefix matches.
    /// The paths need not exist.
    pub fn candidates(&self, path: &str) -> Vec<PathBuf> {
        let Some(p) = Self::clean(path) else { return vec![] };
        let mut out = vec![self.data_dir.join(&p), self.game_dir.join(&p)];
        let ps = p.to_string_lossy().replace('\\', "/");
        out.extend(
            self.extra_roots
                .iter()
                .filter_map(|(prefix, root)| Some(root.join(ps.strip_prefix(prefix.as_str())?))),
        );
        out
    }

    /// Where to read `path` from: the data folder first, then the game,
    /// then the extra roots.
    pub fn resolve_read(&self, path: &str) -> Option<PathBuf> {
        self.candidates(path).into_iter().find(|p| p.exists())
    }

    /// Loads the image table `path` (`name-table-W-H.png` or `name-N.png`
    /// files), from the first place in `candidates` that has it.
    pub fn load_table(&self, path: &str) -> Result<Vec<Bitmap>, ImageError> {
        let path = path.strip_suffix(".pdt").unwrap_or(path);
        let mut err = None;
        for base in self.candidates(path) {
            if !base.parent().is_some_and(|d| d.is_dir()) {
                continue;
            }
            match pdsim_gfx::table::load_table(&base) {
                Ok(v) => return Ok(v),
                Err(e) => err = err.or(Some(e)),
            }
        }
        Err(err.unwrap_or_else(|| ImageError::Other(format!("image table not found: {path}"))))
    }

    /// Like `resolve_read`, trying each extension in turn when `path` has
    /// none that exists.
    pub fn resolve_with_ext(&self, path: &str, exts: &[&str]) -> Option<PathBuf> {
        if let Some(p) = self.resolve_read(path) {
            if p.is_file() {
                return Some(p);
            }
        }
        exts.iter()
            .find_map(|e| self.resolve_read(&format!("{path}.{e}")))
    }

    /// Where to write `path` (always the data folder).
    pub fn resolve_write(&self, path: &str) -> Option<PathBuf> {
        let p = Self::clean(path)?;
        let full = self.data_dir.join(p);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).ok()?;
        }
        Some(full)
    }

    /// Reads a file.
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.resolve_read(path)?).ok()
    }

    /// Writes a file.
    pub fn write(&self, path: &str, data: &[u8]) -> bool {
        self.resolve_write(path)
            .is_some_and(|p| std::fs::write(p, data).is_ok())
    }

    /// Deletes a file in the data folder.
    pub fn delete(&self, path: &str) -> bool {
        Self::clean(path).is_some_and(|p| std::fs::remove_file(self.data_dir.join(p)).is_ok())
    }

    /// Lists a folder (data and game merged). Folders end with `/`.
    pub fn list(&self, path: &str) -> Vec<String> {
        let Some(p) = Self::clean(path) else { return vec![] };
        let mut out: Vec<String> = vec![];
        for base in [&self.data_dir, &self.game_dir] {
            if let Ok(rd) = std::fs::read_dir(base.join(&p)) {
                for e in rd.flatten() {
                    let mut n = e.file_name().to_string_lossy().to_string();
                    if e.path().is_dir() {
                        n.push('/');
                    }
                    if !out.contains(&n) {
                        out.push(n);
                    }
                }
            }
        }
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_escapes() {
        assert!(FileSystem::clean("../x").is_none());
        assert_eq!(FileSystem::clean("/a/./b").unwrap(), PathBuf::from("a/b"));
    }

    #[test]
    fn data_shadows_game() {
        let root = std::env::temp_dir().join(format!("pdsim-fs-{}", std::process::id()));
        let fs = FileSystem::new(root.join("game"), root.join("data"));
        std::fs::create_dir_all(&fs.game_dir).unwrap();
        std::fs::write(fs.game_dir.join("f.txt"), "game").unwrap();
        assert_eq!(fs.read("f.txt").unwrap(), b"game");
        assert!(fs.write("f.txt", b"data"));
        assert_eq!(fs.read("f.txt").unwrap(), b"data");
        assert_eq!(fs.list(""), vec!["f.txt".to_string()]);
        assert!(fs.delete("f.txt"));
        assert_eq!(fs.read("f.txt").unwrap(), b"game");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tables_load_from_extra_roots() {
        let root = std::env::temp_dir().join(format!("pdsim-fs-table-{}", std::process::id()));
        let mut fs = FileSystem::new(root.join("game"), root.join("data"));
        std::fs::create_dir_all(&fs.game_dir).unwrap();
        let sdk = root.join("sdk/CoreLibs");
        std::fs::create_dir_all(sdk.join("assets/crank")).unwrap();
        let png = Bitmap::new(8, 2, pdsim_gfx::Color::White).encode_png().unwrap();
        std::fs::write(sdk.join("assets/crank/frames-table-4-2.png"), png).unwrap();
        assert!(fs.load_table("CoreLibs/assets/crank/frames").is_err());
        fs.extra_roots.push(("CoreLibs/".into(), sdk));
        assert_eq!(fs.load_table("CoreLibs/assets/crank/frames").unwrap().len(), 2);
        assert_eq!(
            fs.load_table("CoreLibs/assets/crank/frames.pdt").unwrap().len(),
            2
        );
        std::fs::remove_dir_all(root).ok();
    }
}
