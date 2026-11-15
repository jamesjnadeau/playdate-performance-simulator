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

/// Which CPU emulator runs a C game (exists when C games are supported).
#[cfg(any(feature = "c-games", feature = "thumb"))]
pub use pdsim_arm::Backend as CpuBackend;

/// The CPU a build uses unless told otherwise: Unicorn when it is compiled
/// in, otherwise the Thumb interpreter.
#[cfg(feature = "c-games")]
pub const DEFAULT_CPU: &str = "unicorn";
#[cfg(not(feature = "c-games"))]
pub const DEFAULT_CPU: &str = "thumb";

#[cfg(feature = "c-games")]
fn default_cpu() -> CpuBackend {
    CpuBackend::Unicorn
}

#[cfg(all(feature = "thumb", not(feature = "c-games")))]
fn default_cpu() -> CpuBackend {
    CpuBackend::Thumb
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
    /// The CPU emulator for C games.
    #[cfg(any(feature = "c-games", feature = "thumb"))]
    pub cpu: CpuBackend,
}

impl RunConfig {
    /// Picks the CPU emulator by name (`unicorn` or `thumb`). `Err` carries
    /// the name when this build doesn't include that backend.
    #[cfg_attr(not(any(feature = "c-games", feature = "thumb")), allow(unused_variables))]
    pub fn set_cpu(&mut self, name: &str) -> Result<(), String> {
        match name {
            #[cfg(feature = "c-games")]
            "unicorn" => {
                self.cpu = CpuBackend::Unicorn;
                Ok(())
            }
            #[cfg(feature = "thumb")]
            "thumb" => {
                self.cpu = CpuBackend::Thumb;
                Ok(())
            }
            _ => Err(name.to_string()),
        }
    }
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            calibration: Calibration::default_estimates(),
            data_dir: None,
            sdk: Device::sdk_from_env(),
            profile: true,
            #[cfg(any(feature = "c-games", feature = "thumb"))]
            cpu: default_cpu(),
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

/// Shown when a C game is loaded without an SDK. The browser UI matches on
/// "Load your SDK folder" in the emscripten text.
#[cfg(all(any(feature = "c-games", feature = "thumb"), not(target_os = "emscripten")))]
const SDK_NEEDED: &str =
    "C games need the Playdate SDK's C_API headers. Set PLAYDATE_SDK_PATH or pass --sdk.";
#[cfg(all(any(feature = "c-games", feature = "thumb"), target_os = "emscripten"))]
const SDK_NEEDED: &str =
    "C games need the Playdate SDK's C_API headers. Load your SDK folder (it is kept in this browser).";

#[cfg(not(any(feature = "c-games", feature = "thumb")))]
fn load_c(
    _dev: pdsim_device::SharedDevice,
    _elf: &Path,
    _cfg: &RunConfig,
) -> Result<Box<dyn Game>, GameError> {
    Err(GameError(
        "C games are not supported in this build (built without the c-games feature)".into(),
    ))
}

#[cfg(any(feature = "c-games", feature = "thumb"))]
fn load_c(dev: pdsim_device::SharedDevice, elf: &Path, cfg: &RunConfig) -> Result<Box<dyn Game>, GameError> {
    if cfg.sdk.is_none() {
        // The API layout is read from the SDK's C_API headers. The native
        // build looks at PLAYDATE_SDK_PATH (via RunConfig::default) or --sdk.
        return Err(GameError(SDK_NEEDED.to_string()));
    }
    Ok(Box::new(pdsim_arm::ArmGame::load(
        dev,
        elf,
        cfg.sdk.as_deref(),
        cfg.profile,
        cfg.cpu,
    )?))
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

    #[cfg(not(any(feature = "c-games", feature = "thumb")))]
    #[test]
    fn c_games_need_the_feature() {
        let dir = std::env::temp_dir().join(format!("pdsim-runner-no-c-games-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pdex.elf"), b"").unwrap();
        let err = load(&dir, &RunConfig::default()).err().unwrap();
        assert!(err.0.contains("c-games feature"), "{}", err.0);
    }

    #[cfg(any(feature = "c-games", feature = "thumb"))]
    #[test]
    fn c_game_without_sdk_names_the_sdk() {
        let ex = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/c-stress");
        let cfg = RunConfig {
            sdk: None,
            data_dir: Some(
                std::env::temp_dir().join(format!("pdsim-runner-no-sdk-data-{}", std::process::id())),
            ),
            ..RunConfig::default()
        };
        let err = load(&ex, &cfg).err().expect("loading without an SDK must fail");
        #[cfg(target_os = "emscripten")]
        assert!(err.0.contains("Load your SDK folder"), "{}", err.0);
        #[cfg(not(target_os = "emscripten"))]
        assert!(err.0.contains("PLAYDATE_SDK_PATH"), "{}", err.0);
    }
}
