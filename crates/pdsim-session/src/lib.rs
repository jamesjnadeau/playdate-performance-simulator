//! A simulation session: one game on its own thread.
//!
//! The UI sends [`Command`]s; the session sends back events (JSON values with
//! a `type` field) through the `emit` callback given to [`Session::spawn`].
//!
//! In **device-speed** mode the session paces real time to device time, so a
//! game runs, stutters and drops frames as it would on hardware. In
//! **turbo** mode it runs as fast as the host allows and still reports
//! device timings.

mod core;
pub use core::{SessionCore, Tick};

use pdsim_core::HotspotTable;
use serde::Deserialize;
use serde_json::Value;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Commands a UI can send.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Command {
    /// Load (or reload) a game.
    Load {
        path: String,
        calibration: Option<String>,
    },
    Play,
    Pause,
    /// Run exactly one frame (pauses first).
    Step,
    /// "device" or "turbo".
    SetMode {
        mode: String,
    },
    /// Device-speed multiplier (1.0 = real device speed).
    SetSpeed {
        speed: f64,
    },
    Button {
        name: String,
        down: bool,
    },
    Crank {
        delta: f32,
    },
    CrankTo {
        angle: f32,
    },
    Dock {
        docked: bool,
    },
    Accel {
        x: f32,
        y: f32,
        z: f32,
    },
    BreakOnOverBudget {
        on: bool,
    },
    /// Write a Chrome trace of the recorded frames.
    ExportTrace {
        path: Option<String>,
    },
    /// Ask for the flame data of a recorded frame (latest if `None`).
    Flame {
        index: Option<u64>,
    },
    /// A UI connected: resend the current state.
    Hello,
    Quit,
}

/// Handle to a running session thread.
pub struct Session {
    tx: Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    /// Starts a session thread. `emit(event)` is called from that thread.
    pub fn spawn<F>(emit: F) -> Session
    where
        F: Fn(Value) + Send + 'static,
    {
        let (tx, rx) = channel();
        let thread = std::thread::Builder::new()
            .name("pdsim-session".into())
            .spawn(move || run_thread(Box::new(emit), rx))
            .expect("spawn session thread");
        Session {
            tx,
            thread: Some(thread),
        }
    }

    /// Sends a command.
    pub fn send(&self, c: Command) {
        let _ = self.tx.send(c);
    }

    /// Sends a command given as JSON.
    pub fn send_json(&self, v: Value) -> Result<(), String> {
        let c: Command = serde_json::from_value(v).map_err(|e| e.to_string())?;
        self.send(c);
        Ok(())
    }
}

/// The session thread: drives a [`SessionCore`] from a command queue.
fn run_thread(emit: Box<dyn Fn(Value)>, rx: Receiver<Command>) {
    let start = Instant::now();
    let now_ms = || start.elapsed().as_secs_f64() * 1000.0;
    let mut core = SessionCore::new(emit);
    loop {
        let now = now_ms();
        let timeout = match core.tick(now).next_wake_ms {
            Some(w) => Duration::from_secs_f64((w - now).max(0.0) / 1000.0),
            None => Duration::from_millis(200),
        };
        match rx.recv_timeout(timeout) {
            Ok(c) => {
                let quit = c == Command::Quit;
                core.handle(c, now_ms());
                if quit {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Keeps the hotspot type reachable for users of this crate.
pub type Hotspots = HotspotTable;

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::sync::{Arc, Mutex};

    fn example() -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/lua-stress")
            .display()
            .to_string()
    }

    fn wait_for(events: &Arc<Mutex<Vec<Value>>>, ty: &str, timeout: Duration) -> Option<Value> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Some(v) = events.lock().unwrap().iter().find(|e| e["type"] == ty) {
                return Some(v.clone());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn device_speed_runs_in_real_time_and_turbo_is_faster() {
        let events = Arc::new(Mutex::new(vec![]));
        let ev = events.clone();
        let s = Session::spawn(move |v| ev.lock().unwrap().push(v));
        s.send(Command::Load {
            path: example(),
            calibration: None,
        });
        assert!(
            wait_for(&events, "loaded", Duration::from_secs(20)).is_some(),
            "{:?}",
            events.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(500));
        s.send(Command::Pause);
        std::thread::sleep(Duration::from_millis(100));
        let frames = events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "frame")
            .count();
        // ~15 frames at 30 fps in 0.5 s (allow slack for slow hosts).
        assert!(
            (5..=20).contains(&frames),
            "device-speed frames in 0.5 s: {frames}"
        );
        events.lock().unwrap().clear();
        s.send(Command::SetMode { mode: "turbo".into() });
        s.send(Command::Play);
        std::thread::sleep(Duration::from_millis(500));
        s.send(Command::Pause);
        std::thread::sleep(Duration::from_millis(100));
        let turbo = events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "frame")
            .count();
        assert!(turbo > frames, "turbo {turbo} vs device {frames}");
        let stats = wait_for(&events, "stats", Duration::from_secs(1)).unwrap();
        assert!(stats["hotspots"].as_array().unwrap().len() > 1);
    }

    #[test]
    fn breaks_on_over_budget_frame_and_steps() {
        let events = Arc::new(Mutex::new(vec![]));
        let ev = events.clone();
        let s = Session::spawn(move |v| ev.lock().unwrap().push(v));
        s.send(Command::BreakOnOverBudget { on: true });
        s.send(Command::SetMode { mode: "turbo".into() });
        s.send(Command::Load {
            path: example(),
            calibration: None,
        });
        let b = wait_for(&events, "break", Duration::from_secs(30));
        assert!(b.is_some(), "no break event");
        let flame = wait_for(&events, "flame", Duration::from_secs(1)).unwrap();
        assert_eq!(flame["index"], 59);
        assert!(flame["folded"]
            .as_object()
            .unwrap()
            .keys()
            .any(|k| k.contains("spike")));
        events.lock().unwrap().clear();
        s.send(Command::Step);
        let f = wait_for(&events, "frame", Duration::from_secs(5)).unwrap();
        assert_eq!(f["index"], 60);
        let fb = base64::engine::general_purpose::STANDARD
            .decode(f["fb"].as_str().unwrap())
            .unwrap();
        assert_eq!(fb.len(), 52 * 240);
    }
}
