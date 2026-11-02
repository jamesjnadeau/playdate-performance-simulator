//! Calibration profiles: the per-unit device costs the simulator charges.
//!
//! A profile is a JSON file. The built-in [`Calibration::default_estimates`]
//! are educated guesses; `tools/fit-calibration` produces a fitted profile
//! from benchmark runs on a real device.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The built-in estimates, used until device data exists.
pub const DEFAULT_CALIBRATION_JSON: &str = include_str!("../../../calibration/default.json");

/// Errors loading a calibration profile.
#[derive(Debug, thiserror::Error)]
pub enum CalibrationError {
    /// The file could not be read.
    #[error("reading calibration file: {0}")]
    Io(#[from] std::io::Error),
    /// The file is not a valid profile.
    #[error("parsing calibration file: {0}")]
    Parse(#[from] serde_json::Error),
    /// The profile is internally inconsistent.
    #[error("invalid calibration: {0}")]
    Invalid(String),
}

/// A device calibration profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Calibration {
    /// Human readable name.
    pub name: String,
    /// Device revision the profile describes ("rev-a", "rev-b").
    pub device: String,
    /// True when the numbers are estimates rather than fitted device data.
    #[serde(default)]
    pub estimated: bool,
    /// CPU clock in Hz.
    pub cpu_hz: u64,
    /// Device RAM in bytes.
    pub ram_bytes: u64,
    /// Default display refresh rate in frames per second.
    #[serde(default = "default_refresh")]
    pub default_refresh_rate: f32,
    /// Lua VM costs.
    pub lua: LuaCosts,
    /// ARM instruction class costs.
    pub arm: ArmCosts,
    /// Cost of each API call, keyed by `area.function` (for example
    /// `graphics.fillRect`).
    pub api: BTreeMap<String, ApiCost>,
    /// Cost used for API calls missing from [`Calibration::api`].
    pub api_default: ApiCost,
    /// Display transfer cost.
    pub display: DisplayCost,
}

fn default_refresh() -> f32 {
    30.0
}

/// Lua VM costs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LuaCosts {
    /// Cycles per executed opcode, keyed by opcode name (`MOVE`, `ADD`, ...).
    pub opcodes: BTreeMap<String, f64>,
    /// Cost of an opcode missing from `opcodes`.
    pub default_opcode: f64,
    /// GC cycles charged per byte allocated.
    pub gc_per_alloc_byte: f64,
    /// GC cycles charged per byte freed.
    pub gc_per_free_byte: f64,
    /// GC cycles per unit of collector work (as reported by the patched VM).
    pub gc_per_work_unit: f64,
    /// Overhead of a call from Lua into a native API function.
    pub native_call_overhead: f64,
}

/// ARM Cortex-M7 instruction class costs, in cycles.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArmCosts {
    /// Cycles per instruction class (`alu`, `mul`, `div`, `load`, `store`,
    /// `load_multiple`, `branch`, `fp`, `fp_div`, `other`).
    pub classes: BTreeMap<String, f64>,
    /// Extra cycles per register for LDM/STM/PUSH/POP.
    pub per_register: f64,
    /// Cost of a class missing from `classes`.
    pub default_class: f64,
}

/// Display transfer costs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DisplayCost {
    /// Fixed cost of a display update.
    pub base: f64,
    /// Cost per changed row.
    pub per_row: f64,
}

/// Cost model for one API function:
/// `base + per_pixel·pixels + per_row·rows + per_byte·bytes + per_item·items`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiCost {
    #[serde(default)]
    pub base: f64,
    #[serde(default)]
    pub per_pixel: f64,
    #[serde(default)]
    pub per_row: f64,
    #[serde(default)]
    pub per_byte: f64,
    #[serde(default)]
    pub per_item: f64,
}

/// Work done by an API call, measured by the implementation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    /// Pixels touched.
    pub pixels: u64,
    /// Framebuffer rows touched.
    pub rows: u64,
    /// Bytes read, written or copied.
    pub bytes: u64,
    /// Items processed (sprites, glyphs, polygon points, ...).
    pub items: u64,
}

impl Work {
    /// Work with only a pixel and row count.
    pub fn pixels(pixels: u64, rows: u64) -> Self {
        Work {
            pixels,
            rows,
            ..Default::default()
        }
    }

    /// Adds another work measurement to this one.
    pub fn add(&mut self, o: Work) {
        self.pixels += o.pixels;
        self.rows += o.rows;
        self.bytes += o.bytes;
        self.items += o.items;
    }
}

impl ApiCost {
    /// Cycles for a call that did `w` work.
    pub fn cycles(&self, w: Work) -> f64 {
        self.base
            + self.per_pixel * w.pixels as f64
            + self.per_row * w.rows as f64
            + self.per_byte * w.bytes as f64
            + self.per_item * w.items as f64
    }
}

impl Default for Calibration {
    fn default() -> Self {
        Self::default_estimates()
    }
}

impl Calibration {
    /// The built-in estimated profile.
    pub fn default_estimates() -> Self {
        Self::from_json(DEFAULT_CALIBRATION_JSON).expect("built-in calibration is valid")
    }

    /// Parses a profile from JSON.
    pub fn from_json(s: &str) -> Result<Self, CalibrationError> {
        let c: Calibration = serde_json::from_str(s)?;
        c.validate()?;
        Ok(c)
    }

    /// Loads a profile from a file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CalibrationError> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }

    /// Serializes the profile as pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("calibration serializes")
    }

    fn validate(&self) -> Result<(), CalibrationError> {
        if self.cpu_hz == 0 {
            return Err(CalibrationError::Invalid("cpu_hz must be > 0".into()));
        }
        if !(0.0..=50.0).contains(&self.default_refresh_rate) {
            return Err(CalibrationError::Invalid(
                "default_refresh_rate must be 0..=50".into(),
            ));
        }
        let neg = |v: f64| !v.is_finite() || v < 0.0;
        for (k, v) in &self.lua.opcodes {
            if neg(*v) {
                return Err(CalibrationError::Invalid(format!(
                    "lua opcode {k} has invalid cost {v}"
                )));
            }
        }
        for (k, v) in &self.api {
            if [v.base, v.per_pixel, v.per_row, v.per_byte, v.per_item]
                .iter()
                .any(|x| neg(*x))
            {
                return Err(CalibrationError::Invalid(format!("api {k} has an invalid cost")));
            }
        }
        Ok(())
    }

    /// Cost of a Lua opcode by name.
    pub fn lua_opcode(&self, name: &str) -> f64 {
        self.lua
            .opcodes
            .get(name)
            .copied()
            .unwrap_or(self.lua.default_opcode)
    }

    /// Cost of an ARM instruction class by name.
    pub fn arm_class(&self, name: &str) -> f64 {
        self.arm
            .classes
            .get(name)
            .copied()
            .unwrap_or(self.arm.default_class)
    }

    /// Cost model for an API function, falling back to `api_default`.
    pub fn api(&self, name: &str) -> ApiCost {
        self.api.get(name).copied().unwrap_or(self.api_default)
    }

    /// Cycles for an API call.
    pub fn api_cycles(&self, name: &str, w: Work) -> f64 {
        self.api(name).cycles(w)
    }

    /// Cycles to push `rows` changed rows to the display.
    pub fn display_cycles(&self, rows: u64) -> f64 {
        if rows == 0 {
            0.0
        } else {
            self.display.base + self.display.per_row * rows as f64
        }
    }

    /// Converts cycles to milliseconds of device time.
    pub fn cycles_to_ms(&self, cycles: u64) -> f64 {
        cycles as f64 * 1000.0 / self.cpu_hz as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_loads() {
        let c = Calibration::default_estimates();
        assert_eq!(c.cpu_hz, 168_000_000);
        assert!(c.estimated);
        assert!(c.lua_opcode("MOVE") > 0.0);
        assert_eq!(c.lua_opcode("NOT_AN_OPCODE"), c.lua.default_opcode);
        assert!(c.api.contains_key("graphics.fillRect"));
    }

    #[test]
    fn api_cost_formula() {
        let a = ApiCost {
            base: 10.0,
            per_pixel: 0.5,
            per_row: 2.0,
            per_byte: 1.0,
            per_item: 3.0,
        };
        let w = Work {
            pixels: 100,
            rows: 10,
            bytes: 4,
            items: 2,
        };
        assert_eq!(a.cycles(w), 10.0 + 50.0 + 20.0 + 4.0 + 6.0);
    }

    #[test]
    fn round_trips_json() {
        let c = Calibration::default_estimates();
        let back = Calibration::from_json(&c.to_json()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn rejects_negative_costs() {
        let mut c = Calibration::default_estimates();
        c.lua.opcodes.insert("MOVE".into(), -1.0);
        assert!(Calibration::from_json(&c.to_json()).is_err());
    }

    #[test]
    fn display_cost_is_zero_without_rows() {
        let c = Calibration::default_estimates();
        assert_eq!(c.display_cycles(0), 0.0);
        assert!(c.display_cycles(240) > c.display_cycles(1));
    }

    #[test]
    fn cycles_to_ms() {
        let c = Calibration::default_estimates();
        assert!((c.cycles_to_ms(168_000) - 1.0).abs() < 1e-9);
    }
}
