//! Scripted input for headless runs.
//!
//! ```json
//! { "events": [
//!     { "frame": 10, "press": ["a"] },
//!     { "frame": 12, "release": ["a"] },
//!     { "frame": 20, "crank": 15.0 },
//!     { "frame": 30, "crank_to": 90.0, "dock": false }
//! ], "crank_per_frame": 0.0 }
//! ```

use crate::input::{buttons, Input};
use serde::{Deserialize, Serialize};

/// One scripted input event.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ScriptEvent {
    pub frame: u64,
    #[serde(default)]
    pub press: Vec<String>,
    #[serde(default)]
    pub release: Vec<String>,
    /// Turn the crank by this many degrees.
    #[serde(default)]
    pub crank: Option<f32>,
    /// Turn the crank to this angle.
    #[serde(default)]
    pub crank_to: Option<f32>,
    #[serde(default)]
    pub dock: Option<bool>,
    #[serde(default)]
    pub accel: Option<(f32, f32, f32)>,
}

/// A whole input script.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct InputScript {
    #[serde(default)]
    pub events: Vec<ScriptEvent>,
    /// Degrees to turn the crank every frame.
    #[serde(default)]
    pub crank_per_frame: f32,
}

impl InputScript {
    pub fn from_json(s: &str) -> serde_json::Result<Self> {
        serde_json::from_str(s)
    }

    /// Applies the events for `frame` (before `Input::begin_frame`).
    pub fn apply(&self, frame: u64, input: &mut Input) {
        if self.crank_per_frame != 0.0 {
            input.crank_by(self.crank_per_frame);
        }
        for e in self.events.iter().filter(|e| e.frame == frame) {
            for b in e.press.iter().filter_map(|n| buttons::from_name(n)) {
                input.press(b);
            }
            for b in e.release.iter().filter_map(|n| buttons::from_name(n)) {
                input.release(b);
            }
            if let Some(d) = e.dock {
                input.set_docked(d);
            }
            if let Some(c) = e.crank {
                input.crank_by(c);
            }
            if let Some(a) = e.crank_to {
                input.crank_to(a);
            }
            if let Some((x, y, z)) = e.accel {
                input.set_accel(x, y, z);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_events_on_their_frame() {
        let s = InputScript::from_json(
            r#"{"events":[{"frame":2,"press":["a","up"]},{"frame":3,"release":["a"]}]}"#,
        )
        .unwrap();
        let mut i = Input::new();
        for f in 0..4 {
            s.apply(f, &mut i);
            i.begin_frame();
            match f {
                2 => assert_eq!(i.pushed, buttons::A | buttons::UP),
                3 => {
                    assert_eq!(i.released, buttons::A);
                    assert_eq!(i.current, buttons::UP);
                }
                _ => {}
            }
        }
    }
}
