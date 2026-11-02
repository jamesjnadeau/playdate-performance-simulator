//! Chrome trace / Perfetto JSON export.
//!
//! Load the output in `chrome://tracing` or <https://ui.perfetto.dev>. Times
//! are device time, not host time.

use crate::scheduler::FrameStats;
use serde_json::{json, Value};

/// Builds a Chrome trace event list from frame stats.
///
/// Each frame becomes a slice on the "frames" track with its categories laid
/// out as nested slices in order; the idle remainder of the frame is shown on
/// the same track. Memory and CPU use are emitted as counters, and hot
/// functions as slices on a "hotspots" track.
pub fn chrome_trace(frames: &[FrameStats]) -> Value {
    let mut events = vec![
        json!({"ph": "M", "pid": 1, "name": "process_name", "args": {"name": "Playdate (simulated)"}}),
        json!({"ph": "M", "pid": 1, "tid": 1, "name": "thread_name", "args": {"name": "frames"}}),
        json!({"ph": "M", "pid": 1, "tid": 2, "name": "thread_name", "args": {"name": "hotspots"}}),
    ];
    for f in frames {
        let ts = f.start_ms * 1000.0;
        events.push(json!({
            "ph": "X", "pid": 1, "tid": 1, "ts": ts, "dur": f.work_ms * 1000.0,
            "name": format!("frame {}", f.index),
            "cat": if f.over_budget { "frame,over_budget" } else { "frame" },
            "args": {"work_ms": f.work_ms, "budget_ms": f.budget_ms, "dropped": f.dropped, "rows_updated": f.rows_updated},
        }));
        let mut t = ts;
        for (cat, ms) in &f.categories {
            if *ms <= 0.0 {
                continue;
            }
            events.push(json!({"ph": "X", "pid": 1, "tid": 1, "ts": t, "dur": ms * 1000.0, "name": cat.name(), "cat": cat.name()}));
            t += ms * 1000.0;
        }
        // Hotspots for the frame, widest first, laid end to end.
        let mut hot: Vec<(&String, &u64)> = f.profile.folded.iter().collect();
        hot.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let total_cycles = f.cycles.total().max(1) as f64;
        let mut t = ts;
        for (stack, cycles) in hot.into_iter().take(32) {
            let dur = f.work_ms * 1000.0 * (*cycles as f64 / total_cycles);
            let leaf = stack.rsplit(';').next().unwrap_or(stack);
            events.push(json!({"ph": "X", "pid": 1, "tid": 2, "ts": t, "dur": dur, "name": leaf, "args": {"stack": stack, "cycles": cycles}}));
            t += dur;
        }
        events.push(json!({"ph": "C", "pid": 1, "ts": ts, "name": "memory", "args": {
            "lua_heap": f.memory.lua_heap, "images": f.memory.images, "c_heap": f.memory.c_heap}}));
        events.push(json!({"ph": "C", "pid": 1, "ts": ts, "name": "cpu %", "args": {"cpu": f.cpu_percent}}));
    }
    json!({"traceEvents": events, "displayTimeUnit": "ms"})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::Calibration;
    use crate::ledger::{Category, CategoryCycles};
    use crate::profile::FrameProfile;
    use crate::scheduler::{MemoryStats, Scheduler};

    #[test]
    fn trace_has_frames_and_categories() {
        let cal = Calibration::default_estimates();
        let mut s = Scheduler::new(&cal);
        let mut c = CategoryCycles::default();
        c.add(Category::Lua, 168_000);
        c.add(Category::Gfx, 336_000);
        let mut p = FrameProfile::default();
        p.add_stack(&["main.lua:1 update"], 100_000);
        let f = s.end_frame(c, 0, MemoryStats::default(), p);
        let t = chrome_trace(&[f]);
        let ev = t["traceEvents"].as_array().unwrap();
        assert!(ev
            .iter()
            .any(|e| e["name"] == "frame 0" && (e["dur"].as_f64().unwrap() - 3000.0).abs() < 1e-6));
        let gfx = ev.iter().find(|e| e["name"] == "gfx").unwrap();
        assert!((gfx["ts"].as_f64().unwrap() - 1000.0).abs() < 1e-6);
        assert!(ev
            .iter()
            .any(|e| e["name"] == "main.lua:1 update" && e["tid"] == 2));
    }
}
