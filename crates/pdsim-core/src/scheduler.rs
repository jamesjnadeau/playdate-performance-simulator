//! The frame scheduler: turns charged cycles into device frame timings.

use crate::calibration::Calibration;
use crate::ledger::{Category, CategoryCycles};
use crate::profile::FrameProfile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Memory in use at the end of a frame, in bytes.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryStats {
    /// Lua heap.
    pub lua_heap: u64,
    /// Images, fonts and other graphics resources.
    pub images: u64,
    /// Native heap (C games).
    pub c_heap: u64,
    /// Device RAM.
    pub capacity: u64,
}

impl MemoryStats {
    /// Total bytes in use.
    pub fn used(&self) -> u64 {
        self.lua_heap + self.images + self.c_heap
    }
}

/// Timing and resource data for one simulated frame.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrameStats {
    /// Frame number, starting at 0.
    pub index: u64,
    /// Device time at which the frame started, in ms.
    pub start_ms: f64,
    /// Time the CPU was busy this frame, in ms.
    pub work_ms: f64,
    /// Time until the next frame starts, in ms (a multiple of the budget when
    /// a refresh rate is set).
    pub frame_ms: f64,
    /// Budget per frame, in ms (0 when the refresh rate is unlimited).
    pub budget_ms: f64,
    /// Busy time as a percentage of the frame time.
    pub cpu_percent: f64,
    /// Busy time per category, in ms.
    pub categories: BTreeMap<Category, f64>,
    /// Cycles per category.
    pub cycles: CategoryCycles,
    /// Whether the work exceeded the budget.
    pub over_budget: bool,
    /// Display refreshes missed because the frame ran long.
    pub dropped: u32,
    /// Display rows sent this frame.
    pub rows_updated: u32,
    /// Memory at frame end.
    pub memory: MemoryStats,
    /// Hotspot samples for this frame.
    #[serde(default, skip_serializing_if = "FrameProfile::is_empty")]
    pub profile: FrameProfile,
}

/// Advances the virtual device clock frame by frame.
#[derive(Debug, Clone)]
pub struct Scheduler {
    cpu_hz: u64,
    refresh_rate: f32,
    frame_index: u64,
    /// Virtual time at the start of the current frame, in cycles.
    now_cycles: u64,
    dropped_total: u64,
    over_budget_total: u64,
}

impl Scheduler {
    /// A scheduler for the device described by `cal`.
    pub fn new(cal: &Calibration) -> Self {
        Scheduler {
            cpu_hz: cal.cpu_hz,
            refresh_rate: cal.default_refresh_rate,
            frame_index: 0,
            now_cycles: 0,
            dropped_total: 0,
            over_budget_total: 0,
        }
    }

    /// Sets the refresh rate (`playdate.display.setRefreshRate`). The device
    /// caps it at 50; 0 means "as fast as possible".
    pub fn set_refresh_rate(&mut self, rate: f32) {
        self.refresh_rate = if rate.is_finite() {
            rate.clamp(0.0, 50.0)
        } else {
            30.0
        };
    }

    /// Current refresh rate.
    pub fn refresh_rate(&self) -> f32 {
        self.refresh_rate
    }

    /// Cycles available per frame, or `None` when unlimited.
    pub fn budget_cycles(&self) -> Option<u64> {
        if self.refresh_rate > 0.0 {
            Some((self.cpu_hz as f64 / self.refresh_rate as f64).round() as u64)
        } else {
            None
        }
    }

    /// Index of the frame in progress.
    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }

    /// Device time at the start of the current frame, in cycles.
    pub fn now_cycles(&self) -> u64 {
        self.now_cycles
    }

    /// Device time at the start of the current frame, in ms.
    pub fn now_ms(&self) -> f64 {
        self.cycles_to_ms(self.now_cycles)
    }

    /// Total refreshes dropped so far.
    pub fn dropped_total(&self) -> u64 {
        self.dropped_total
    }

    /// Total over-budget frames so far.
    pub fn over_budget_total(&self) -> u64 {
        self.over_budget_total
    }

    fn cycles_to_ms(&self, c: u64) -> f64 {
        c as f64 * 1000.0 / self.cpu_hz as f64
    }

    /// Finishes the current frame and advances the clock.
    ///
    /// With a refresh rate the device waits for the next refresh, so a frame
    /// lasts a whole number of refresh periods: work that overruns one period
    /// by any amount costs a full extra period (a dropped frame).
    pub fn end_frame(
        &mut self,
        cycles: CategoryCycles,
        rows_updated: u32,
        memory: MemoryStats,
        profile: FrameProfile,
    ) -> FrameStats {
        let work = cycles.total();
        let (frame_cycles, dropped, over, budget_ms) = match self.budget_cycles() {
            Some(budget) => {
                let periods = work.div_ceil(budget).max(1);
                (
                    periods * budget,
                    (periods - 1) as u32,
                    work > budget,
                    self.cycles_to_ms(budget),
                )
            }
            None => (work.max(1), 0, false, 0.0),
        };
        let categories = cycles.iter().map(|(c, n)| (c, self.cycles_to_ms(n))).collect();
        let stats = FrameStats {
            index: self.frame_index,
            start_ms: self.now_ms(),
            work_ms: self.cycles_to_ms(work),
            frame_ms: self.cycles_to_ms(frame_cycles),
            budget_ms,
            cpu_percent: 100.0 * work as f64 / frame_cycles as f64,
            categories,
            cycles,
            over_budget: over,
            dropped,
            rows_updated,
            memory,
            profile,
        };
        self.frame_index += 1;
        self.now_cycles += frame_cycles;
        self.dropped_total += dropped as u64;
        self.over_budget_total += over as u64;
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work(c: u64) -> CategoryCycles {
        let mut cc = CategoryCycles::default();
        cc.add(Category::Lua, c);
        cc
    }

    fn sched() -> Scheduler {
        Scheduler::new(&Calibration::default_estimates())
    }

    #[test]
    fn budget_at_30fps() {
        let s = sched();
        assert_eq!(s.budget_cycles(), Some(5_600_000));
    }

    #[test]
    fn under_budget_frame_takes_one_period() {
        let mut s = sched();
        let f = s.end_frame(
            work(2_800_000),
            0,
            MemoryStats::default(),
            FrameProfile::default(),
        );
        assert!(!f.over_budget);
        assert_eq!(f.dropped, 0);
        assert!((f.frame_ms - 33.333).abs() < 0.01);
        assert!((f.work_ms - 16.666).abs() < 0.01);
        assert!((f.cpu_percent - 50.0).abs() < 1e-9);
        assert_eq!(s.frame_index(), 1);
        assert!((s.now_ms() - 33.333).abs() < 0.01);
    }

    #[test]
    fn over_budget_frame_drops_refreshes() {
        let mut s = sched();
        let f = s.end_frame(
            work(5_600_001),
            0,
            MemoryStats::default(),
            FrameProfile::default(),
        );
        assert!(f.over_budget);
        assert_eq!(f.dropped, 1);
        assert!((f.frame_ms - 66.666).abs() < 0.01);
        let f = s.end_frame(
            work(5_600_000 * 3),
            0,
            MemoryStats::default(),
            FrameProfile::default(),
        );
        assert_eq!(f.dropped, 2);
        assert_eq!(s.dropped_total(), 3);
        assert_eq!(s.over_budget_total(), 2);
    }

    #[test]
    fn exact_budget_is_not_over() {
        let mut s = sched();
        let f = s.end_frame(
            work(5_600_000),
            0,
            MemoryStats::default(),
            FrameProfile::default(),
        );
        assert!(!f.over_budget);
        assert_eq!(f.dropped, 0);
    }

    #[test]
    fn refresh_rate_is_clamped_and_zero_is_unlimited() {
        let mut s = sched();
        s.set_refresh_rate(100.0);
        assert_eq!(s.refresh_rate(), 50.0);
        assert_eq!(s.budget_cycles(), Some(3_360_000));
        s.set_refresh_rate(0.0);
        assert_eq!(s.budget_cycles(), None);
        let f = s.end_frame(
            work(1_680_000),
            0,
            MemoryStats::default(),
            FrameProfile::default(),
        );
        assert!((f.frame_ms - 10.0).abs() < 1e-9);
        assert!(!f.over_budget);
    }

    #[test]
    fn doubling_work_doubles_work_time() {
        let mut s = sched();
        let a = s.end_frame(
            work(1_000_000),
            0,
            MemoryStats::default(),
            FrameProfile::default(),
        );
        let b = s.end_frame(
            work(2_000_000),
            0,
            MemoryStats::default(),
            FrameProfile::default(),
        );
        assert!((b.work_ms / a.work_ms - 2.0).abs() < 1e-9);
    }
}
