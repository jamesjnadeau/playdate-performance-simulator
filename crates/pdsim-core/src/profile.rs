//! Hotspot attribution: where the cycles went, per function.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Cycles attributed to call stacks during one frame.
///
/// Stacks are stored "folded": frames joined by `;`, outermost first, the
/// format used by flame graph tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FrameProfile {
    /// Folded stack -> self cycles.
    pub folded: HashMap<String, u64>,
}

impl FrameProfile {
    /// Attributes `cycles` to the stack `stack` (outermost first).
    pub fn add_stack<S: AsRef<str>>(&mut self, stack: &[S], cycles: u64) {
        if cycles == 0 || stack.is_empty() {
            return;
        }
        let key = stack.iter().map(|s| s.as_ref()).collect::<Vec<_>>().join(";");
        *self.folded.entry(key).or_insert(0) += cycles;
    }

    /// Attributes cycles to an already folded stack.
    pub fn add_folded(&mut self, folded: &str, cycles: u64) {
        if cycles > 0 {
            *self.folded.entry(folded.to_string()).or_insert(0) += cycles;
        }
    }

    /// Total attributed cycles.
    pub fn total(&self) -> u64 {
        self.folded.values().sum()
    }

    /// Merges another profile into this one.
    pub fn merge(&mut self, o: &FrameProfile) {
        for (k, v) in &o.folded {
            *self.folded.entry(k.clone()).or_insert(0) += v;
        }
    }

    /// Whether nothing was attributed.
    pub fn is_empty(&self) -> bool {
        self.folded.is_empty()
    }
}

/// One row of the hotspot table.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Hotspot {
    /// Function name (`source:line function` for Lua, symbol for C).
    pub name: String,
    /// Cycles spent in the function itself.
    pub self_cycles: u64,
    /// Cycles spent in the function and everything it called.
    pub total_cycles: u64,
}

/// Cumulative per-function totals built from frame profiles.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HotspotTable {
    self_cycles: HashMap<String, u64>,
    total_cycles: HashMap<String, u64>,
}

impl HotspotTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a frame's samples.
    pub fn add(&mut self, p: &FrameProfile) {
        for (stack, cycles) in &p.folded {
            let frames: Vec<&str> = stack.split(';').collect();
            if let Some(leaf) = frames.last() {
                *self.self_cycles.entry(leaf.to_string()).or_insert(0) += cycles;
            }
            // Count each function once per stack, even when recursive.
            let mut seen: Vec<&str> = Vec::with_capacity(frames.len());
            for f in frames {
                if !seen.contains(&f) {
                    seen.push(f);
                    *self.total_cycles.entry(f.to_string()).or_insert(0) += cycles;
                }
            }
        }
    }

    /// The `n` functions with the most self cycles.
    pub fn top(&self, n: usize) -> Vec<Hotspot> {
        let mut v: Vec<Hotspot> = self
            .total_cycles
            .iter()
            .map(|(name, total)| Hotspot {
                name: name.clone(),
                self_cycles: self.self_cycles.get(name).copied().unwrap_or(0),
                total_cycles: *total,
            })
            .collect();
        v.sort_by(|a, b| {
            b.self_cycles
                .cmp(&a.self_cycles)
                .then(b.total_cycles.cmp(&a.total_cycles))
                .then(a.name.cmp(&b.name))
        });
        v.truncate(n);
        v
    }

    /// Forgets everything.
    pub fn clear(&mut self) {
        self.self_cycles.clear();
        self.total_cycles.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_and_total_cycles() {
        let mut p = FrameProfile::default();
        p.add_stack(&["main", "update", "draw"], 100);
        p.add_stack(&["main", "update"], 30);
        p.add_stack(&["main", "update", "physics"], 50);
        let mut t = HotspotTable::new();
        t.add(&p);
        let top = t.top(10);
        assert_eq!(top[0].name, "draw");
        assert_eq!(top[0].self_cycles, 100);
        let update = top.iter().find(|h| h.name == "update").unwrap();
        assert_eq!(update.self_cycles, 30);
        assert_eq!(update.total_cycles, 180);
        assert_eq!(p.total(), 180);
    }

    #[test]
    fn recursion_counts_total_once() {
        let mut p = FrameProfile::default();
        p.add_stack(&["fib", "fib", "fib"], 10);
        let mut t = HotspotTable::new();
        t.add(&p);
        assert_eq!(t.top(1)[0].total_cycles, 10);
    }
}
