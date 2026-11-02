//! Runs Playdate Lua games on the simulated device.
//!
//! The game runs on a vendored Lua 5.4 patched to count executed opcodes
//! (see `vendor/README.md`). The counts, multiplied by the calibrated cost of
//! each opcode, are charged to the device's cycle ledger; API calls charge
//! their own calibrated costs.

pub mod api;
pub mod corelibs;
pub mod ctx;
pub mod game;
pub mod preprocess;
pub mod vm;

pub use game::{source_dir, LuaGame, LuaOptions};
