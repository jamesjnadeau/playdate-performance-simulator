//! Finds out what kind of game a path holds and loads it.

use pdsim_core::Calibration;
use pdsim_device::{Device, Game, GameError};
use pdsim_lua::{LuaGame, LuaOptions};
use std::path::{Path, PathBuf};

/// What kind of game a path holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameKind {
    /// A Lua game; the path is its source folder (with `main.lua`).
    Lua(PathBuf),
    /// A C game; the path is its device build (`pdex.elf`).
    C(PathBuf),
}

impl GameKind {
    /// The folder the game reads its files from.
    pub fn game_dir(&self) -> PathBuf {
        match self {
            GameKind::Lua(d) => d.clone(),
            GameKind::C(elf) => elf.parent().map(Path::to_path_buf).unwrap_or_default(),
        }
    }
}

/// Detects the game in `path`: a `main.lua` folder (or its parent with a
/// `Source/` folder), a `pdex.elf`, or a folder containing one.
pub fn detect(path: &Path) -> Result<GameKind, GameError> {
    if path.is_file() {
        if path.extension().is_some_and(|e| e == "elf") {
            return Ok(GameKind::C(path.to_path_buf()));
        }
        if path.file_name().is_some_and(|n| n == "main.lua") {
            return Ok(GameKind::Lua(
                path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            ));
        }
    }
    for dir in [path.to_path_buf(), path.join("Source")] {
        if dir.join("main.lua").is_file() {
            return Ok(GameKind::Lua(dir));
        }
        if dir.join("pdex.elf").is_file() {
            return Ok(GameKind::C(dir.join("pdex.elf")));
        }
    }
    Err(GameError(format!(
        "{}: no main.lua or pdex.elf found (expected a game folder, its Source/ folder, or a pdex.elf)",
        path.display()
    )))
}

/// How to run a game.
#[derive(Debug, Clone)]
pub struct RunConfig {
    pub calibration: Calibration,
    /// Where the game's saved data goes. Defaults to a folder in the
    /// system temp directory.
    pub data_dir: Option<PathBuf>,
    /// Playdate SDK, for CoreLibs, the system font and C headers.
    pub sdk: Option<PathBuf>,
    /// Attribute cycles to functions.
    pub profile: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            calibration: Calibration::default_estimates(),
            data_dir: None,
            sdk: Device::sdk_from_env(),
            profile: true,
        }
    }
}

/// Loads the game at `path`.
pub fn load(path: &Path, cfg: &RunConfig) -> Result<Box<dyn Game>, GameError> {
    let kind = detect(path)?;
    let game_dir = kind.game_dir();
    let data_dir = cfg.data_dir.clone().unwrap_or_else(|| {
        let name = game_dir
            .canonicalize()
            .ok()
            .and_then(|p| {
                let p = if p.ends_with("Source") {
                    p.parent().map(Path::to_path_buf).unwrap_or(p)
                } else {
                    p
                };
                p.file_name().map(|n| n.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "game".into());
        std::env::temp_dir().join("pdsim-data").join(name)
    });
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| GameError(format!("creating {}: {e}", data_dir.display())))?;
    let dev = Device::new(cfg.calibration.clone(), &game_dir, &data_dir, cfg.sdk.clone()).shared();
    match kind {
        GameKind::Lua(_) => {
            let g = LuaGame::load(
                dev,
                LuaOptions {
                    profile: cfg.profile,
                    ..Default::default()
                },
            )?;
            Ok(Box::new(g))
        }
        GameKind::C(elf) => load_c(dev, &elf, cfg),
    }
}

fn load_c(
    _dev: pdsim_device::SharedDevice,
    elf: &Path,
    _cfg: &RunConfig,
) -> Result<Box<dyn Game>, GameError> {
    Err(GameError(format!(
        "{}: C games are not supported yet",
        elf.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_example() {
        let ex = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lua-stress");
        assert!(matches!(detect(&ex).unwrap(), GameKind::Lua(p) if p.ends_with("Source")));
        assert!(detect(Path::new("/nonexistent")).is_err());
    }
}
