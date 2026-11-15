//! The steppable heart of a session: all state and behaviour, no threads and
//! no clock of its own. Hosts feed it commands and the current time in
//! milliseconds, and call [`SessionCore::tick`] when it asks to be woken.

use crate::Command;
use base64::Engine;
use pdsim_core::{trace, Calibration, Category, FrameStats};
use pdsim_device::{buttons, Game};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::PathBuf;

/// How many frames of history to keep (for traces and the flame view).
const HISTORY: usize = 3000;
/// Stats are sent at most this often.
const STATS_INTERVAL_MS: f64 = 100.0;

/// What a host should do after a [`SessionCore::tick`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    /// When to call `tick` again (host clock, ms). `None` means idle until
    /// the next command; in turbo mode it is the current time.
    pub next_wake_ms: Option<f64>,
}

/// A simulated Playdate session that is driven by its host.
pub struct SessionCore {
    emit: Box<dyn Fn(Value)>,
    game: Option<Box<dyn Game>>,
    running: bool,
    turbo: bool,
    speed: f64,
    break_on_over: bool,
    history: VecDeque<FrameStats>,
    pending: Vec<Value>,
    last_stats_ms: f64,
    // Pacing anchor: device ms at `anchor_wall_ms`.
    anchor_wall_ms: f64,
    anchor_device_ms: f64,
    /// Wall time at which the next frame is due (device-speed mode).
    next_frame_ms: f64,
    log_sent: usize,
    path: Option<String>,
    calibration: Calibration,
    loaded_event: Option<Value>,
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

impl SessionCore {
    /// `emit(event)` receives every event (JSON with a `type` field).
    pub fn new(emit: Box<dyn Fn(Value)>) -> SessionCore {
        SessionCore {
            emit,
            game: None,
            running: false,
            turbo: false,
            speed: 1.0,
            break_on_over: false,
            history: VecDeque::new(),
            pending: vec![],
            last_stats_ms: 0.0,
            anchor_wall_ms: 0.0,
            anchor_device_ms: 0.0,
            next_frame_ms: 0.0,
            log_sent: 0,
            path: None,
            calibration: Calibration::default_estimates(),
            loaded_event: None,
        }
    }

    /// Runs at most one frame if the session is running and one is due at
    /// `now_ms`, and says when to call again.
    pub fn tick(&mut self, now_ms: f64) -> Tick {
        if !(self.running && self.game.is_some()) {
            return Tick { next_wake_ms: None };
        }
        if self.turbo {
            self.frame(now_ms);
            return Tick {
                next_wake_ms: (self.running && self.game.is_some()).then_some(now_ms),
            };
        }
        if now_ms >= self.next_frame_ms {
            self.frame(now_ms);
            self.schedule_next(now_ms);
        }
        Tick {
            next_wake_ms: (self.running && self.game.is_some()).then_some(self.next_frame_ms),
        }
    }

    fn schedule_next(&mut self, now_ms: f64) {
        let Some(g) = &self.game else { return };
        let device_ms = g.device().borrow().scheduler.now_ms();
        let mut target = self.anchor_wall_ms + ((device_ms - self.anchor_device_ms) / self.speed).max(0.0);
        if now_ms - target > 250.0 {
            // The host fell behind; don't try to catch up in a burst.
            self.reanchor(now_ms);
            target = now_ms;
        }
        self.next_frame_ms = target;
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

    fn reanchor(&mut self, now_ms: f64) {
        self.anchor_wall_ms = now_ms;
        self.next_frame_ms = now_ms;
        self.anchor_device_ms = self
            .game
            .as_ref()
            .map_or(0.0, |g| g.device().borrow().scheduler.now_ms());
    }

    /// Applies a command. `now_ms` is the host clock in milliseconds.
    pub fn handle(&mut self, c: Command, now_ms: f64) {
        match c {
            Command::Load { path, calibration } => self.load(path, calibration, now_ms),
            Command::Play => {
                self.running = true;
                self.reanchor(now_ms);
                self.state();
            }
            Command::Pause => {
                self.running = false;
                self.flush_stats(now_ms);
                self.state();
            }
            Command::Step => {
                self.running = false;
                if self.game.is_some() {
                    self.frame(now_ms);
                    self.flush_stats(now_ms);
                    self.send_flame(None);
                }
                self.state();
            }
            Command::SetMode { mode } => {
                self.turbo = mode == "turbo";
                self.reanchor(now_ms);
                self.state();
            }
            Command::SetSpeed { speed } => {
                self.speed = speed.clamp(0.05, 20.0);
                self.reanchor(now_ms);
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
            Command::Quit => {
                self.game = None;
                self.running = false;
            }
        }
    }

    fn load(&mut self, path: String, calibration: Option<String>, now_ms: f64) {
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
                self.reanchor(now_ms);
                self.send_log();
            }
            Err(e) => self.error(e.0),
        }
        self.state();
    }

    fn error(&mut self, msg: String) {
        (self.emit)(json!({"type": "error", "message": msg}));
    }

    fn frame(&mut self, now_ms: f64) {
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
                if now_ms - self.last_stats_ms >= STATS_INTERVAL_MS {
                    self.flush_stats(now_ms);
                }
                if over && self.break_on_over {
                    self.running = false;
                    self.flush_stats(now_ms);
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

    fn flush_stats(&mut self, now_ms: f64) {
        self.last_stats_ms = now_ms;
        self.send_log();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn core() -> (SessionCore, Rc<RefCell<Vec<Value>>>) {
        let events = Rc::new(RefCell::new(vec![]));
        let ev = events.clone();
        (
            SessionCore::new(Box::new(move |v| ev.borrow_mut().push(v))),
            events,
        )
    }

    fn frames(ev: &[Value]) -> usize {
        ev.iter().filter(|e| e["type"] == "frame").count()
    }

    fn load_example(c: &mut SessionCore) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/lua-stress")
            .display()
            .to_string();
        c.handle(
            Command::Load {
                path,
                calibration: None,
            },
            0.0,
        );
    }

    #[test]
    fn idle_core_never_wakes() {
        let (mut c, ev) = core();
        assert_eq!(c.tick(0.0).next_wake_ms, None);
        assert!(ev.borrow().is_empty());
    }

    #[test]
    fn device_mode_runs_one_frame_per_due_tick() {
        let (mut c, ev) = core();
        load_example(&mut c);
        let t = c.tick(0.0);
        assert_eq!(frames(&ev.borrow()), 1);
        let w = t.next_wake_ms.unwrap();
        assert!(w > 0.0);
        assert_eq!(c.tick(w - 1.0).next_wake_ms, Some(w));
        assert_eq!(frames(&ev.borrow()), 1);
        c.tick(w);
        assert_eq!(frames(&ev.borrow()), 2);
    }

    #[test]
    fn command_during_wait_keeps_the_schedule() {
        let (mut c, ev) = core();
        load_example(&mut c);
        let w = c.tick(0.0).next_wake_ms.unwrap();
        let n = frames(&ev.borrow());
        c.handle(
            Command::Button {
                name: "a".into(),
                down: true,
            },
            w / 2.0,
        );
        assert_eq!(c.tick(w / 2.0).next_wake_ms, Some(w));
        assert_eq!(frames(&ev.borrow()), n);
    }

    #[test]
    fn host_falling_behind_reanchors() {
        let (mut c, ev) = core();
        load_example(&mut c);
        let w = c.tick(0.0).next_wake_ms.unwrap();
        let n = frames(&ev.borrow());
        let t = c.tick(w + 1000.0);
        assert_eq!(frames(&ev.borrow()), n + 1);
        assert!(t.next_wake_ms.unwrap() >= w + 1000.0);
    }

    #[test]
    fn turbo_wakes_immediately() {
        let (mut c, _ev) = core();
        load_example(&mut c);
        c.handle(Command::SetMode { mode: "turbo".into() }, 0.0);
        assert_eq!(c.tick(5.0).next_wake_ms, Some(5.0));
    }

    #[test]
    fn pause_goes_idle_and_step_runs_exactly_one_frame() {
        let (mut c, ev) = core();
        load_example(&mut c);
        c.tick(0.0);
        c.handle(Command::Pause, 0.0);
        let n = frames(&ev.borrow());
        assert_eq!(c.tick(1000.0).next_wake_ms, None);
        assert_eq!(frames(&ev.borrow()), n);
        c.handle(Command::Step, 0.0);
        assert_eq!(frames(&ev.borrow()), n + 1);
        assert_eq!(c.tick(2000.0).next_wake_ms, None);
        assert_eq!(frames(&ev.borrow()), n + 1);
    }
}
