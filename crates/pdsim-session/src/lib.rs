//! A simulation session: one game on its own thread.
//!
//! The UI sends [`Command`]s; the session sends back events (JSON values with
//! a `type` field) through the `emit` callback given to [`Session::spawn`].
//!
//! In **device-speed** mode the session paces real time to device time, so a
//! game runs, stutters and drops frames as it would on hardware. In
//! **turbo** mode it runs as fast as the host allows and still reports
//! device timings.

use base64::Engine;
use pdsim_core::{trace, Calibration, Category, FrameStats, HotspotTable};
use pdsim_device::{buttons, Game};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
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

/// How many frames of history to keep (for traces and the flame view).
const HISTORY: usize = 3000;
/// Stats are sent at most this often.
const STATS_INTERVAL: Duration = Duration::from_millis(100);

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
            .spawn(move || Runner::new(Box::new(emit)).run(rx))
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

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Runner {
    emit: Box<dyn Fn(Value) + Send>,
    game: Option<Box<dyn Game>>,
    running: bool,
    turbo: bool,
    speed: f64,
    break_on_over: bool,
    history: VecDeque<FrameStats>,
    pending: Vec<Value>,
    last_stats: Instant,
    // Pacing anchor: device ms at `anchor_wall`.
    anchor_wall: Instant,
    anchor_device_ms: f64,
    // Real-time measurement baseline, kept apart from the pacing anchor
    // (pacing re-anchors when the host falls behind, which would hide it).
    rt_wall: Instant,
    rt_device_ms: f64,
    log_sent: usize,
    path: Option<String>,
    calibration: Calibration,
    loaded_event: Option<Value>,
}

/// Device ms per wall ms, or `None` if too little wall time passed to tell.
fn realtime_ratio(device_ms: f64, wall: Duration) -> Option<f64> {
    if wall < Duration::from_millis(50) {
        return None;
    }
    Some(device_ms / (wall.as_secs_f64() * 1000.0))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

impl Runner {
    fn new(emit: Box<dyn Fn(Value) + Send>) -> Runner {
        Runner {
            emit,
            game: None,
            running: false,
            turbo: false,
            speed: 1.0,
            break_on_over: false,
            history: VecDeque::new(),
            pending: vec![],
            last_stats: Instant::now(),
            anchor_wall: Instant::now(),
            anchor_device_ms: 0.0,
            rt_wall: Instant::now(),
            rt_device_ms: 0.0,
            log_sent: 0,
            path: None,
            calibration: Calibration::default_estimates(),
            loaded_event: None,
        }
    }

    fn run(mut self, rx: Receiver<Command>) {
        loop {
            // Wait for a command when idle; otherwise just drain the queue.
            let cmd = if self.running && self.game.is_some() {
                match rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    Err(_) => return,
                }
            } else {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(_) => return,
                }
            };
            if let Some(c) = cmd {
                if c == Command::Quit {
                    return;
                }
                self.handle(c);
                continue;
            }
            if self.running && self.game.is_some() {
                self.frame();
                if self.pace(&rx) {
                    return;
                }
            }
        }
    }

    fn state(&self) {
        (self.emit)(json!({
            "type": "state",
            "running": self.running,
            "mode": if self.turbo { "turbo" } else { "device" },
            "speed": self.speed,
            "breakOnOverBudget": self.break_on_over,
            "loaded": self.game.is_some(),
        }));
    }

    fn device_ms(&self) -> f64 {
        self.game
            .as_ref()
            .map_or(0.0, |g| g.device().borrow().scheduler.now_ms())
    }

    /// Restarts pacing and the real-time measurement from now.
    fn reanchor(&mut self) {
        self.reanchor_pacing();
        self.mark_realtime();
    }

    fn reanchor_pacing(&mut self) {
        self.anchor_wall = Instant::now();
        self.anchor_device_ms = self.device_ms();
    }

    /// Starts a new real-time window and a full stats interval with it, so the
    /// next `stats` has enough wall time to measure.
    fn mark_realtime(&mut self) {
        self.last_stats = Instant::now();
        self.rt_wall = Instant::now();
        self.rt_device_ms = self.device_ms();
    }

    fn handle(&mut self, c: Command) {
        match c {
            Command::Load { path, calibration } => self.load(path, calibration),
            Command::Play => {
                self.running = true;
                self.reanchor();
                self.state();
            }
            Command::Pause => {
                self.running = false;
                self.flush_stats();
                self.state();
            }
            Command::Step => {
                self.running = false;
                if self.game.is_some() {
                    self.frame();
                    self.flush_stats();
                    self.send_flame(None);
                }
                self.state();
            }
            Command::SetMode { mode } => {
                self.turbo = mode == "turbo";
                self.reanchor();
                self.state();
            }
            Command::SetSpeed { speed } => {
                self.speed = speed.clamp(0.05, 20.0);
                self.reanchor();
                self.state();
            }
            Command::BreakOnOverBudget { on } => {
                self.break_on_over = on;
                self.state();
            }
            Command::Button { name, down } => {
                if let (Some(g), Some(b)) = (&self.game, buttons::from_name(&name)) {
                    let mut d = g.device().borrow_mut();
                    if down {
                        d.input.press(b)
                    } else {
                        d.input.release(b)
                    }
                }
            }
            Command::Crank { delta } => {
                if let Some(g) = &self.game {
                    g.device().borrow_mut().input.crank_by(delta);
                }
            }
            Command::CrankTo { angle } => {
                if let Some(g) = &self.game {
                    g.device().borrow_mut().input.crank_to(angle);
                }
            }
            Command::Dock { docked } => {
                if let Some(g) = &self.game {
                    g.device().borrow_mut().input.set_docked(docked);
                }
            }
            Command::Accel { x, y, z } => {
                if let Some(g) = &self.game {
                    g.device().borrow_mut().input.set_accel(x, y, z);
                }
            }
            Command::ExportTrace { path } => self.export_trace(path),
            Command::Flame { index } => self.send_flame(index),
            Command::Hello => {
                if let Some(l) = &self.loaded_event {
                    (self.emit)(l.clone());
                }
                self.state();
            }
            Command::Quit => {}
        }
    }

    fn load(&mut self, path: String, calibration: Option<String>) {
        self.game = None;
        self.history.clear();
        self.pending.clear();
        self.log_sent = 0;
        let cal = match calibration.filter(|s| !s.is_empty()) {
            Some(p) => match Calibration::load(&p) {
                Ok(c) => c,
                Err(e) => return self.error(format!("calibration {p}: {e}")),
            },
            None => Calibration::default_estimates(),
        };
        let cfg = pdsim_runner::RunConfig {
            calibration: cal.clone(),
            ..Default::default()
        };
        match pdsim_runner::load(std::path::Path::new(&path), &cfg) {
            Ok(g) => {
                let ev = json!({
                    "type": "loaded",
                    "path": path,
                    "description": g.describe(),
                    "calibration": cal.name,
                    "estimated": cal.estimated,
                    "cpuHz": cal.cpu_hz,
                    "ramBytes": cal.ram_bytes,
                    "categories": Category::ALL.iter().map(|c| c.name()).collect::<Vec<_>>(),
                });
                (self.emit)(ev.clone());
                self.loaded_event = Some(ev);
                self.game = Some(g);
                self.path = Some(path);
                self.calibration = cal;
                self.running = true;
                self.reanchor();
                self.send_log();
            }
            Err(e) => self.error(e.0),
        }
        self.state();
    }

    fn error(&mut self, msg: String) {
        (self.emit)(json!({"type": "error", "message": msg}));
    }

    fn frame(&mut self) {
        let Some(g) = self.game.as_mut() else { return };
        match g.frame(None) {
            Ok(f) => {
                let (fb, rows, inverted) = {
                    let d = g.device().borrow();
                    let mut rows = [0u8; 30];
                    for (i, r) in d.last_rows.iter().enumerate() {
                        if *r {
                            rows[i / 8] |= 1 << (i % 8);
                        }
                    }
                    (b64(&d.gfx.display().data), b64(&rows), d.gfx.inverted)
                };
                (self.emit)(json!({
                    "type": "frame",
                    "index": f.index,
                    "fb": fb,
                    "rows": rows,
                    "inverted": inverted,
                    "overBudget": f.over_budget,
                }));
                self.pending.push(frame_lite(&f));
                let over = f.over_budget;
                self.history.push_back(f);
                while self.history.len() > HISTORY {
                    self.history.pop_front();
                }
                if self.last_stats.elapsed() >= STATS_INTERVAL {
                    self.flush_stats();
                }
                if over && self.break_on_over {
                    self.running = false;
                    self.flush_stats();
                    self.send_flame(None);
                    (self.emit)(json!({"type": "break", "reason": "over budget"}));
                    self.state();
                }
            }
            Err(e) => {
                self.running = false;
                self.send_log();
                self.error(e.0);
                self.state();
            }
        }
    }

    /// Waits until device time catches up with wall time. Returns true on `Quit`.
    fn pace(&mut self, rx: &Receiver<Command>) -> bool {
        if self.turbo {
            return false;
        }
        let Some(g) = &self.game else { return false };
        let device_ms = g.device().borrow().scheduler.now_ms();
        let target = self.anchor_wall
            + Duration::from_secs_f64(((device_ms - self.anchor_device_ms) / self.speed / 1000.0).max(0.0));
        let now = Instant::now();
        if target > now {
            // Sleep, but wake for commands (input should not wait a frame).
            if let Ok(c) = rx.recv_timeout(target - now) {
                if c == Command::Quit {
                    return true;
                }
                self.handle(c);
                // Keep the remaining wait.
                let now = Instant::now();
                if target > now && self.running && !self.turbo {
                    std::thread::sleep(target - now);
                }
            }
        } else if now - target > Duration::from_millis(250) {
            // The host fell behind; don't try to catch up in a burst.
            self.reanchor_pacing();
        }
        false
    }

    fn send_log(&mut self) {
        let Some(g) = &self.game else { return };
        let d = g.device().borrow();
        if self.log_sent > d.log.len() {
            self.log_sent = 0;
        }
        if d.log.len() > self.log_sent {
            let lines: Vec<&String> = d.log[self.log_sent..].iter().collect();
            (self.emit)(json!({"type": "log", "lines": lines}));
            self.log_sent = d.log.len();
        }
    }

    fn flush_stats(&mut self) {
        self.last_stats = Instant::now();
        self.send_log();
        if self.game.is_none() {
            return;
        }
        let realtime = if self.running {
            realtime_ratio(self.device_ms() - self.rt_device_ms, self.rt_wall.elapsed())
        } else {
            None
        };
        self.mark_realtime();
        let Some(g) = &self.game else { return };
        let frames = std::mem::take(&mut self.pending);
        let d = g.device().borrow();
        let hot = d.hotspots.top(40);
        let last = self.history.back();
        (self.emit)(json!({
            "type": "stats",
            "frames": frames,
            "hotspots": hot,
            "droppedTotal": d.scheduler.dropped_total(),
            "overBudgetTotal": d.scheduler.over_budget_total(),
            "frameIndex": d.scheduler.frame_index(),
            "deviceMs": d.scheduler.now_ms(),
            "refreshRate": d.scheduler.refresh_rate(),
            "memory": last.map(|f| f.memory),
            "realtime": realtime,
        }));
    }

    fn send_flame(&mut self, index: Option<u64>) {
        let f = match index {
            Some(i) => self.history.iter().find(|f| f.index == i),
            None => self.history.back(),
        };
        let Some(f) = f else { return };
        (self.emit)(json!({
            "type": "flame",
            "index": f.index,
            "workCycles": f.cycles.total(),
            "folded": f.profile.folded,
        }));
    }

    fn export_trace(&mut self, path: Option<String>) {
        let frames: Vec<FrameStats> = self.history.iter().cloned().collect();
        let t = trace::chrome_trace(&frames);
        let path = path
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let base = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir);
                let n = self
                    .game
                    .as_ref()
                    .map_or(0, |g| g.device().borrow().scheduler.frame_index());
                base.join(format!("pdsim-trace-{n}.json"))
            });
        match std::fs::write(&path, t.to_string()) {
            Ok(()) => (self.emit)(
                json!({"type": "traceExported", "path": path.display().to_string(), "frames": frames.len()}),
            ),
            Err(e) => self.error(format!("writing {}: {e}", path.display())),
        }
    }
}

/// The per-frame data the perf panel needs (no profile).
fn frame_lite(f: &FrameStats) -> Value {
    json!({
        "index": f.index,
        "workMs": f.work_ms,
        "frameMs": f.frame_ms,
        "budgetMs": f.budget_ms,
        "cpu": f.cpu_percent,
        "categories": f.categories,
        "overBudget": f.over_budget,
        "dropped": f.dropped,
        "rows": f.rows_updated,
        "memory": f.memory,
    })
}

/// Keeps the hotspot type reachable for users of this crate.
pub type Hotspots = HotspotTable;

#[cfg(test)]
mod tests {
    use super::*;
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
    fn dropping_a_running_session_returns() {
        let events = Arc::new(Mutex::new(vec![]));
        let ev = events.clone();
        let s = Session::spawn(move |v| ev.lock().unwrap().push(v));
        s.send(Command::Load {
            path: example(),
            calibration: None,
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
        assert_eq!(realtime_ratio(100.0, Duration::from_millis(40)), None);
        assert_eq!(realtime_ratio(100.0, Duration::from_millis(100)), Some(1.0));
        assert_eq!(realtime_ratio(50.0, Duration::from_millis(100)), Some(0.5));
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
