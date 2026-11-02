//! The simulated Playdate: everything a game backend (Lua or C) shares.
//!
//! [`Device`] owns the cost ledger, the frame scheduler, graphics, sprites,
//! input and the file sandbox. A backend implements [`Game`] to run one
//! frame at a time.

pub mod fs;
pub mod input;
pub mod script;

pub use fs::FileSystem;
pub use input::{buttons, Input};
pub use script::{InputScript, ScriptEvent};

use pdsim_core::{
    Calibration, Category, FrameProfile, FrameStats, HotspotTable, Ledger, MemoryStats, Scheduler, Work,
};
use pdsim_gfx::{Graphics, SpriteSystem};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The simulated device.
pub struct Device {
    pub cal: Calibration,
    pub ledger: Ledger,
    pub scheduler: Scheduler,
    pub gfx: Graphics,
    pub sprites: SpriteSystem,
    pub input: Input,
    pub fs: FileSystem,
    /// Samples for the frame in progress.
    pub profile: FrameProfile,
    /// Totals over the run.
    pub hotspots: HotspotTable,
    /// Console output (print, logToConsole).
    pub log: Vec<String>,
    /// Native (C) heap in use.
    pub c_heap: u64,
    /// SDK location, if known.
    pub sdk: Option<PathBuf>,
    elapsed_start: u64,
    /// Rows sent to the display by the last finished frame.
    pub last_rows: [bool; pdsim_core::LCD_HEIGHT],
    /// Set by the game to stop the run (e.g. a fatal error).
    pub stopped: Option<String>,
    /// Stats of the last finished frame.
    pub last_frame: Option<FrameStats>,
}

/// A device shared between a backend and its API bindings.
pub type SharedDevice = Rc<RefCell<Device>>;

impl Device {
    /// A device running the game in `game_dir`, saving data to `data_dir`.
    pub fn new(cal: Calibration, game_dir: &Path, data_dir: &Path, sdk: Option<PathBuf>) -> Device {
        let mut fs = FileSystem::new(game_dir.to_path_buf(), data_dir.to_path_buf());
        if let Some(s) = &sdk {
            fs.extra_roots.push(("CoreLibs/".into(), s.join("CoreLibs")));
        }
        Device {
            scheduler: Scheduler::new(&cal),
            cal,
            ledger: Ledger::new(),
            gfx: Graphics::new(sdk.as_deref()),
            sprites: SpriteSystem::new(),
            input: Input::new(),
            fs,
            profile: FrameProfile::default(),
            hotspots: HotspotTable::new(),
            log: vec![],
            c_heap: 0,
            sdk,
            elapsed_start: 0,
            last_rows: [false; pdsim_core::LCD_HEIGHT],
            stopped: None,
            last_frame: None,
        }
    }

    /// Wraps the device for sharing.
    pub fn shared(self) -> SharedDevice {
        Rc::new(RefCell::new(self))
    }

    /// The SDK path from `PLAYDATE_SDK_PATH`, if set and present.
    pub fn sdk_from_env() -> Option<PathBuf> {
        let p = PathBuf::from(std::env::var_os("PLAYDATE_SDK_PATH")?);
        p.join("C_API").exists().then_some(p)
    }

    /// Current device time in cycles (including work done so far this frame).
    pub fn now_cycles(&self) -> u64 {
        self.scheduler.now_cycles() + self.ledger.frame_total()
    }

    /// Current device time in milliseconds.
    pub fn now_ms(&self) -> f64 {
        self.cal.cycles_to_ms(self.now_cycles())
    }

    /// Charges raw cycles.
    pub fn charge(&mut self, cat: Category, cycles: f64) {
        self.ledger.charge(cat, cycles);
    }

    /// Charges an API call by its calibrated cost; returns the cycles.
    pub fn charge_api(&mut self, cat: Category, name: &str, work: Work) -> f64 {
        let c = self.cal.api_cycles(name, work);
        self.ledger.charge(cat, c);
        c
    }

    /// `resetElapsedTime`.
    pub fn reset_elapsed(&mut self) {
        self.elapsed_start = self.now_cycles();
    }

    /// `getElapsedTime`, in seconds.
    pub fn elapsed_s(&self) -> f64 {
        self.cal
            .cycles_to_ms(self.now_cycles().saturating_sub(self.elapsed_start))
            / 1000.0
    }

    /// Appends a console line.
    pub fn log_line(&mut self, s: impl Into<String>) {
        let s = s.into();
        if self.log.len() > 10_000 {
            self.log.drain(..5_000);
        }
        self.log.push(s);
    }

    /// Starts a frame: applies queued input.
    pub fn begin_frame(&mut self, script: Option<&InputScript>) {
        if let Some(s) = script {
            s.apply(self.scheduler.frame_index(), &mut self.input);
        }
        self.input.begin_frame();
    }

    /// Ends a frame: pushes changed rows to the display (charging the
    /// transfer), advances the clock and returns the frame's stats.
    pub fn end_frame(&mut self, lua_heap: u64) -> FrameStats {
        let rows = self.gfx.flush_display();
        let n = rows.iter().filter(|r| **r).count() as u64;
        let c = self.cal.display_cycles(n);
        self.ledger.charge(Category::Display, c);
        self.last_rows = rows;
        let memory = MemoryStats {
            lua_heap,
            images: self.gfx.image_memory(),
            c_heap: self.c_heap,
            capacity: self.cal.ram_bytes,
        };
        let profile = std::mem::take(&mut self.profile);
        self.hotspots.add(&profile);
        let cycles = self.ledger.take_frame();
        self.scheduler.end_frame(cycles, n as u32, memory, profile)
    }
}

/// Errors from running a game.
#[derive(Debug, Clone)]
pub struct GameError(pub String);

impl std::fmt::Display for GameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GameError {}

/// A game backend.
pub trait Game {
    /// The device the game runs on.
    fn device(&self) -> &SharedDevice;
    /// Runs one frame and returns its stats.
    fn frame(&mut self, script: Option<&InputScript>) -> Result<FrameStats, GameError>;
    /// A short description ("Lua game main.lua", "C game pdex.elf").
    fn describe(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_frame_charges_display_rows() {
        let tmp = std::env::temp_dir();
        let mut d = Device::new(Calibration::default_estimates(), &tmp, &tmp, None);
        d.begin_frame(None);
        d.gfx
            .fill_rect(pdsim_gfx::Rect::new(0, 0, 10, 20), pdsim_gfx::Color::Black);
        let f = d.end_frame(0);
        assert_eq!(f.rows_updated, 20);
        let want = d.cal.display_cycles(20);
        assert_eq!(f.cycles.get(Category::Display), want.round() as u64);
        let f = d.end_frame(0);
        assert_eq!(f.rows_updated, 0);
        assert_eq!(f.cycles.total(), 0);
    }

    #[test]
    fn time_advances_with_work() {
        let tmp = std::env::temp_dir();
        let mut d = Device::new(Calibration::default_estimates(), &tmp, &tmp, None);
        d.reset_elapsed();
        d.charge(Category::Lua, 168_000.0);
        assert!((d.elapsed_s() - 0.001).abs() < 1e-9);
        d.end_frame(0);
        assert!((d.now_ms() - 1000.0 / 30.0).abs() < 1e-6);
    }
}
