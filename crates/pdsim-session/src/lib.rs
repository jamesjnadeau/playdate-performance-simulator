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
        #[serde(default)]
        calibration: Option<String>,
        /// Where the game keeps its save data (default: next to the game).
        #[serde(default, rename = "dataDir")]
        data_dir: Option<String>,
        /// Playdate SDK path (default: from the environment).
        #[serde(default)]
        sdk: Option<String>,
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

/// Native hosts have a filesystem: turns a `traceData` event into the trace
/// file plus `traceExported` (or `error`). Other events pass through (`None`).
fn write_trace(ev: &Value) -> Option<Value> {
    if ev["type"] != "traceData" {
        return None;
    }
    let path = match ev["path"].as_str() {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let base = std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            base.join(format!("pdsim-trace-{}.json", ev["frame"]))
        }
    };
    Some(match std::fs::write(&path, ev["trace"].to_string()) {
        Ok(()) => {
            serde_json::json!({"type": "traceExported", "path": path.display().to_string(), "frames": ev["frames"]})
        }
        Err(e) => serde_json::json!({"type": "error", "message": format!("writing {}: {e}", path.display())}),
    })
}

/// The session thread: drives a [`SessionCore`] from a command queue.
fn run_thread(emit: Box<dyn Fn(Value)>, rx: Receiver<Command>) {
    let start = Instant::now();
    let now_ms = || start.elapsed().as_secs_f64() * 1000.0;
    let mut core = SessionCore::new(Box::new(move |ev| match write_trace(&ev) {
        Some(out) => emit(out),
        None => emit(ev),
    }));
    loop {
        let now = now_ms();
        let timeout = match core.tick(now).next_wake_ms {
            Some(w) => Duration::from_secs_f64((w - now).max(0.0) / 1000.0),
            None => Duration::from_millis(200),
        };
        match rx.recv_timeout(timeout) {
            Ok(c) => {
                if handle_queued(&mut core, c, &rx, &now_ms) {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Handles `first` and every command already queued, so a burst of commands
/// takes effect before the next frame. Returns true after a `Quit`.
fn handle_queued(
    core: &mut SessionCore,
    first: Command,
    rx: &Receiver<Command>,
    now_ms: &dyn Fn() -> f64,
) -> bool {
    let mut next = Some(first);
    while let Some(c) = next {
        let quit = c == Command::Quit;
        core.handle(c, now_ms());
        if quit {
            return true;
        }
        next = rx.try_recv().ok();
    }
    false
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
    fn queued_commands_all_apply_before_the_next_frame() {
        let events = Arc::new(Mutex::new(vec![]));
        let ev = events.clone();
        let mut core = SessionCore::new(Box::new(move |v| ev.lock().unwrap().push(v)));
        let (tx, rx) = channel();
        tx.send(Command::BreakOnOverBudget { on: true }).unwrap();
        tx.send(Command::Hello).unwrap();
        let quit = handle_queued(&mut core, Command::SetMode { mode: "turbo".into() }, &rx, &|| 0.0);
        assert!(!quit);
        let evs = events.lock().unwrap();
        let state = evs.iter().rev().find(|e| e["type"] == "state").expect("state");
        assert_eq!(state["mode"], "turbo");
        assert_eq!(state["breakOnOverBudget"], true);
    }

    #[test]
    fn device_speed_runs_in_real_time_and_turbo_is_faster() {
        let events = Arc::new(Mutex::new(vec![]));
        let ev = events.clone();
        let s = Session::spawn(move |v| ev.lock().unwrap().push(v));
        s.send(Command::Load {
            path: example(),
            calibration: None,
            data_dir: None,
            sdk: None,
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
    fn dropping_a_running_session_returns() {
        let events = Arc::new(Mutex::new(vec![]));
        let ev = events.clone();
        let s = Session::spawn(move |v| ev.lock().unwrap().push(v));
        s.send(Command::Load {
            path: example(),
            calibration: None,
            data_dir: None,
            sdk: None,
        });
        assert!(wait_for(&events, "loaded", Duration::from_secs(20)).is_some());
        std::thread::sleep(Duration::from_millis(200));
        // Device mode sleeps between frames; a quit that lands then must still end the thread.
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(s);
            let _ = done_tx.send(());
        });
        assert!(
            done_rx.recv_timeout(Duration::from_secs(3)).is_ok(),
            "Session::drop hung while running"
        );
    }

    #[test]
    fn stats_while_running_always_carry_realtime() {
        // A (re)start must not send a stats event too soon to measure, or the
        // speed tile blinks '–' on every play.
        let events = Arc::new(Mutex::new(vec![]));
        let ev = events.clone();
        let s = Session::spawn(move |v| ev.lock().unwrap().push(v));
        std::thread::sleep(Duration::from_millis(150));
        s.send(Command::Load {
            path: example(),
            calibration: None,
            data_dir: None,
            sdk: None,
        });
        assert!(wait_for(&events, "loaded", Duration::from_secs(20)).is_some());
        std::thread::sleep(Duration::from_millis(400));
        s.send(Command::Pause);
        std::thread::sleep(Duration::from_millis(150));
        s.send(Command::Play);
        std::thread::sleep(Duration::from_millis(400));
        s.send(Command::Pause);
        std::thread::sleep(Duration::from_millis(100));
        // Each Pause flushes one null (sent just before its `state`); any more
        // came from a running session.
        let nulls = events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "stats" && e["realtime"].is_null())
            .count();
        assert_eq!(nulls, 2, "null realtime stats beyond the two pauses");
    }

    #[test]
    fn realtime_ratio_needs_enough_wall_time() {
        assert_eq!(core::realtime_ratio(100.0, 40.0), None);
        assert_eq!(core::realtime_ratio(100.0, 100.0), Some(1.0));
        assert_eq!(core::realtime_ratio(50.0, 100.0), Some(0.5));
    }

    fn latest_realtime(events: &Arc<Mutex<Vec<Value>>>) -> Option<f64> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "stats")
            .filter_map(|e| e["realtime"].as_f64())
            .next_back()
    }

    #[test]
    fn stats_report_realtime() {
        let events = Arc::new(Mutex::new(vec![]));
        let ev = events.clone();
        let s = Session::spawn(move |v| ev.lock().unwrap().push(v));
        s.send(Command::Load {
            path: example(),
            calibration: None,
            data_dir: None,
            sdk: None,
        });
        assert!(wait_for(&events, "loaded", Duration::from_secs(20)).is_some());
        std::thread::sleep(Duration::from_millis(600));
        let device = latest_realtime(&events).expect("no realtime in device mode");
        assert!((0.7..=1.3).contains(&device), "device-mode realtime {device}");

        events.lock().unwrap().clear();
        s.send(Command::SetMode { mode: "turbo".into() });
        std::thread::sleep(Duration::from_millis(600));
        let turbo = latest_realtime(&events).expect("no realtime in turbo");
        assert!(turbo > 1.5, "turbo realtime {turbo}");

        events.lock().unwrap().clear();
        s.send(Command::Pause);
        let paused = wait_for(&events, "stats", Duration::from_secs(1)).unwrap();
        assert!(paused["realtime"].is_null(), "{paused}");
    }

    #[test]
    fn breaks_on_over_budget_frame_and_steps() {
        let events = Arc::new(Mutex::new(vec![]));
        let ev = events.clone();
        let s = Session::spawn(move |v| ev.lock().unwrap().push(v));
        s.send(Command::BreakOnOverBudget { on: true });
        s.send(Command::SetMode { mode: "turbo".into() });
        // The estimates keep lua-stress under budget until its frame-59
        // spike; under the fitted profile every frame is over.
        let estimates = concat!(env!("CARGO_MANIFEST_DIR"), "/../../calibration/default.json");
        s.send(Command::Load {
            path: example(),
            calibration: Some(estimates.into()),
            data_dir: None,
            sdk: None,
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

    #[test]
    fn native_session_still_writes_trace_files() {
        let events = Arc::new(Mutex::new(vec![]));
        let ev = events.clone();
        let s = Session::spawn(move |v| ev.lock().unwrap().push(v));
        s.send(Command::Load {
            path: example(),
            calibration: None,
            data_dir: None,
            sdk: None,
        });
        assert!(wait_for(&events, "frame", Duration::from_secs(20)).is_some());
        let out = std::env::temp_dir().join(format!("pdsim-trace-test-{}.json", std::process::id()));
        s.send(Command::ExportTrace {
            path: Some(out.display().to_string()),
        });
        let e = wait_for(&events, "traceExported", Duration::from_secs(5)).expect("traceExported");
        assert_eq!(e["path"], out.display().to_string());
        let t: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert!(t["traceEvents"].is_array());
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn load_json_without_new_fields_still_parses() {
        let c = serde_json::from_value::<Command>(serde_json::json!({"type":"load","path":"x"}));
        assert!(c.is_ok());
        let c = serde_json::from_value::<Command>(
            serde_json::json!({"type":"load","path":"x","dataDir":"d","sdk":"s"}),
        )
        .unwrap();
        assert!(matches!(c, Command::Load { data_dir: Some(d), sdk: Some(_), .. } if d == "d"));
    }
}
