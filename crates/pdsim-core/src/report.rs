//! Run summaries for the headless runner and CI.

use crate::ledger::Category;
use crate::profile::{Hotspot, HotspotTable};
use crate::scheduler::FrameStats;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Summary of a simulation run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Report {
    /// Calibration profile used.
    pub calibration: String,
    /// Whether that profile was estimated rather than device-fitted.
    pub estimated_calibration: bool,
    /// Frames simulated.
    pub frames: u64,
    /// Device time simulated, in ms.
    pub device_time_ms: f64,
    /// Frame budget in ms (0 = unlimited).
    pub budget_ms: f64,
    /// Average CPU busy time per frame.
    pub avg_work_ms: f64,
    /// Median CPU busy time.
    pub p50_work_ms: f64,
    /// 95th percentile CPU busy time.
    pub p95_work_ms: f64,
    /// Worst CPU busy time.
    pub max_work_ms: f64,
    /// Frame index of the worst frame.
    pub max_work_frame: u64,
    /// Frames whose work exceeded the budget.
    pub over_budget_frames: u64,
    /// Display refreshes dropped.
    pub dropped_frames: u64,
    /// Frames shown per second of device time.
    pub effective_fps: f64,
    /// Average busy time per category, in ms.
    pub avg_category_ms: BTreeMap<Category, f64>,
    /// Peak memory use in bytes.
    pub peak_memory: u64,
    /// Indices of over-budget frames (at most 1000).
    pub over_budget_frame_indices: Vec<u64>,
    /// Top functions by self time.
    pub hotspots: Vec<Hotspot>,
}

impl Report {
    /// Builds a report from per-frame stats.
    pub fn build(cal_name: &str, estimated: bool, frames: &[FrameStats], hotspots: &HotspotTable) -> Report {
        let n = frames.len().max(1) as f64;
        let mut works: Vec<f64> = frames.iter().map(|f| f.work_ms).collect();
        works.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pct = |p: f64| -> f64 {
            if works.is_empty() {
                0.0
            } else {
                let i = ((works.len() - 1) as f64 * p).round() as usize;
                works[i]
            }
        };
        let (max_work_frame, max_work_ms) = frames
            .iter()
            .map(|f| (f.index, f.work_ms))
            .fold((0, 0.0), |acc, x| if x.1 > acc.1 { x } else { acc });
        let device_time_ms: f64 = frames.iter().map(|f| f.frame_ms).sum();
        let mut avg_category_ms = BTreeMap::new();
        for c in Category::ALL {
            let s: f64 = frames
                .iter()
                .map(|f| f.categories.get(&c).copied().unwrap_or(0.0))
                .sum();
            avg_category_ms.insert(c, s / n);
        }
        Report {
            calibration: cal_name.to_string(),
            estimated_calibration: estimated,
            frames: frames.len() as u64,
            device_time_ms,
            budget_ms: frames.last().map(|f| f.budget_ms).unwrap_or(0.0),
            avg_work_ms: works.iter().sum::<f64>() / n,
            p50_work_ms: pct(0.5),
            p95_work_ms: pct(0.95),
            max_work_ms,
            max_work_frame,
            over_budget_frames: frames.iter().filter(|f| f.over_budget).count() as u64,
            dropped_frames: frames.iter().map(|f| f.dropped as u64).sum(),
            effective_fps: if device_time_ms > 0.0 {
                frames.len() as f64 * 1000.0 / device_time_ms
            } else {
                0.0
            },
            avg_category_ms,
            peak_memory: frames.iter().map(|f| f.memory.used()).max().unwrap_or(0),
            over_budget_frame_indices: frames
                .iter()
                .filter(|f| f.over_budget)
                .map(|f| f.index)
                .take(1000)
                .collect(),
            hotspots: hotspots.top(25),
        }
    }

    /// A short human readable summary.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        use std::fmt::Write;
        let _ = writeln!(
            s,
            "{} frames, {:.1} ms device time, budget {:.2} ms{}",
            self.frames,
            self.device_time_ms,
            self.budget_ms,
            if self.estimated_calibration {
                " (ESTIMATED calibration)"
            } else {
                ""
            }
        );
        let _ = writeln!(
            s,
            "work avg {:.2} ms  p50 {:.2}  p95 {:.2}  max {:.2} (frame {})",
            self.avg_work_ms, self.p50_work_ms, self.p95_work_ms, self.max_work_ms, self.max_work_frame
        );
        let _ = writeln!(
            s,
            "over budget: {} frames, dropped refreshes: {}, effective fps {:.1}",
            self.over_budget_frames, self.dropped_frames, self.effective_fps
        );
        let cats: Vec<String> = self
            .avg_category_ms
            .iter()
            .filter(|(_, v)| **v > 0.0)
            .map(|(k, v)| format!("{k} {v:.2}"))
            .collect();
        let _ = writeln!(s, "avg per category (ms): {}", cats.join(", "));
        let _ = writeln!(s, "peak memory: {:.1} KB", self.peak_memory as f64 / 1024.0);
        if !self.hotspots.is_empty() {
            let _ = writeln!(s, "hotspots (self cycles):");
            for h in self.hotspots.iter().take(10) {
                let _ = writeln!(s, "  {:>12}  {}", h.self_cycles, h.name);
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::Calibration;
    use crate::ledger::CategoryCycles;
    use crate::profile::FrameProfile;
    use crate::scheduler::{MemoryStats, Scheduler};

    #[test]
    fn report_aggregates() {
        let cal = Calibration::default_estimates();
        let mut s = Scheduler::new(&cal);
        let mut frames = vec![];
        for i in 0..10u64 {
            let mut c = CategoryCycles::default();
            c.add(Category::Lua, if i == 7 { 7_000_000 } else { 1_000_000 });
            c.add(Category::Gfx, 500_000);
            frames.push(s.end_frame(
                c,
                10,
                MemoryStats {
                    lua_heap: 1000 * i,
                    ..Default::default()
                },
                FrameProfile::default(),
            ));
        }
        let r = Report::build(&cal.name, cal.estimated, &frames, &HotspotTable::new());
        assert_eq!(r.frames, 10);
        assert_eq!(r.over_budget_frames, 1);
        assert_eq!(r.over_budget_frame_indices, vec![7]);
        assert_eq!(r.dropped_frames, 1);
        assert_eq!(r.max_work_frame, 7);
        assert_eq!(r.peak_memory, 9000);
        assert!((r.effective_fps - 10.0 / (11.0 * 33.3333 / 1000.0)).abs() < 0.01);
        assert!(r.summary().contains("over budget: 1 frames"));
    }
}
