//! The virtual cycle ledger.

use serde::{Deserialize, Serialize};
use std::fmt;

/// What a charge was spent on. Frame time is broken down by these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    /// Lua bytecode execution.
    Lua,
    /// Native (ARM) game code.
    C,
    /// Graphics and sprite API work.
    Gfx,
    /// Lua garbage collection and allocation.
    Gc,
    /// Display transfer.
    Display,
    /// Sound.
    Audio,
    /// File and datastore IO.
    Io,
    /// Everything else (system API calls, timers, ...).
    Other,
}

impl Category {
    /// All categories, in display order.
    pub const ALL: [Category; 8] = [
        Category::Lua,
        Category::C,
        Category::Gfx,
        Category::Gc,
        Category::Display,
        Category::Audio,
        Category::Io,
        Category::Other,
    ];

    /// Number of categories.
    pub const COUNT: usize = Self::ALL.len();

    /// Short lowercase name.
    pub fn name(self) -> &'static str {
        match self {
            Category::Lua => "lua",
            Category::C => "c",
            Category::Gfx => "gfx",
            Category::Gc => "gc",
            Category::Display => "display",
            Category::Audio => "audio",
            Category::Io => "io",
            Category::Other => "other",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Cycles per category.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryCycles(pub [u64; Category::COUNT]);

impl CategoryCycles {
    /// Cycles charged to `cat`.
    pub fn get(&self, cat: Category) -> u64 {
        self.0[cat.index()]
    }

    /// Adds cycles to `cat`.
    pub fn add(&mut self, cat: Category, cycles: u64) {
        self.0[cat.index()] = self.0[cat.index()].saturating_add(cycles);
    }

    /// Sum over all categories.
    pub fn total(&self) -> u64 {
        self.0.iter().sum()
    }

    /// Iterates `(category, cycles)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (Category, u64)> + '_ {
        Category::ALL.iter().map(move |c| (*c, self.get(*c)))
    }
}

/// Accumulates the device cycles spent in the current frame.
///
/// Fractional costs (for example 0.05 cycles per pixel) are accumulated
/// exactly and only rounded when the frame is taken.
#[derive(Debug, Clone, Default)]
pub struct Ledger {
    frame: [f64; Category::COUNT],
    lifetime: CategoryCycles,
}

impl Ledger {
    /// A new, empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Charges `cycles` to `cat`.
    pub fn charge(&mut self, cat: Category, cycles: f64) {
        if cycles > 0.0 && cycles.is_finite() {
            self.frame[cat.index()] += cycles;
        }
    }

    /// Charges a whole number of cycles to `cat`.
    pub fn charge_cycles(&mut self, cat: Category, cycles: u64) {
        self.frame[cat.index()] += cycles as f64;
    }

    /// Cycles charged so far this frame, all categories.
    pub fn frame_total(&self) -> u64 {
        self.frame.iter().sum::<f64>().round() as u64
    }

    /// Cycles charged so far this frame to `cat`.
    pub fn frame_cycles(&self, cat: Category) -> u64 {
        self.frame[cat.index()].round() as u64
    }

    /// Ends the frame: returns the cycles charged and starts a new frame.
    pub fn take_frame(&mut self) -> CategoryCycles {
        let mut out = CategoryCycles::default();
        for cat in Category::ALL {
            let c = self.frame[cat.index()].round() as u64;
            out.add(cat, c);
            self.lifetime.add(cat, c);
        }
        self.frame = [0.0; Category::COUNT];
        out
    }

    /// Everything charged in finished frames.
    pub fn lifetime(&self) -> CategoryCycles {
        self.lifetime
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charges_accumulate_per_category() {
        let mut l = Ledger::new();
        l.charge_cycles(Category::Lua, 100);
        l.charge(Category::Gfx, 0.5);
        l.charge(Category::Gfx, 0.5);
        l.charge_cycles(Category::Lua, 50);
        assert_eq!(l.frame_cycles(Category::Lua), 150);
        assert_eq!(l.frame_cycles(Category::Gfx), 1);
        assert_eq!(l.frame_total(), 151);
        let f = l.take_frame();
        assert_eq!(f.get(Category::Lua), 150);
        assert_eq!(f.total(), 151);
        assert_eq!(l.frame_total(), 0);
        assert_eq!(l.lifetime().get(Category::Lua), 150);
    }

    #[test]
    fn negative_and_nan_charges_are_ignored() {
        let mut l = Ledger::new();
        l.charge(Category::C, -5.0);
        l.charge(Category::C, f64::NAN);
        assert_eq!(l.frame_total(), 0);
    }
}
