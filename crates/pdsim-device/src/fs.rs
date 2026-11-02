//! The game's file system: the read-only game folder and a writable data
//! folder (where `playdate.datastore` and `playdate.file` writes go).

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

    /// Where to read `path` from: the data folder first, then the game.
    pub fn resolve_read(&self, path: &str) -> Option<PathBuf> {
        let p = Self::clean(path)?;
        let d = self.data_dir.join(&p);
        if d.exists() {
            return Some(d);
        }
        let g = self.game_dir.join(&p);
        if g.exists() {
            return Some(g);
        }
        let ps = p.to_string_lossy().replace('\\', "/");
        self.extra_roots.iter().find_map(|(prefix, root)| {
            let rest = ps.strip_prefix(prefix.as_str())?;
            let e = root.join(rest);
            e.exists().then_some(e)
        })
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
}
