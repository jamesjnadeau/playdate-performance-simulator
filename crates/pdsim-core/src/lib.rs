//! Core of the Playdate performance simulator.
//!
//! Nothing in the simulator is timed with the host's wall clock. All work a
//! game does is charged, in device CPU cycles, to a [`Ledger`]. The
//! [`Scheduler`] turns the cycles charged during a frame into device time,
//! applies the display refresh budget, and produces [`FrameStats`].
//!
//! The per-unit costs come from a [`Calibration`] profile, which is either the
//! built-in estimates or a file fitted from benchmark runs on a real device.

pub mod calibration;
pub mod ledger;
pub mod profile;
pub mod report;
pub mod scheduler;
pub mod trace;

pub use calibration::{ApiCost, Calibration, CalibrationError, Work};
pub use ledger::{Category, CategoryCycles, Ledger};
pub use profile::{FrameProfile, HotspotTable};
pub use report::Report;
pub use scheduler::{FrameStats, MemoryStats, Scheduler};

/// Width of the Playdate screen in pixels.
pub const LCD_WIDTH: usize = 400;
/// Height of the Playdate screen in pixels.
pub const LCD_HEIGHT: usize = 240;
/// Bytes per framebuffer row (the device uses 52, with 2 bytes of padding).
pub const LCD_ROWSIZE: usize = 52;
/// Device RAM.
pub const DEVICE_RAM: u64 = 16 * 1024 * 1024;
