//! The simulator session as a plain, single-threaded object: JSON in, a queue
//! of JSON events out. `main.rs` exposes it through a four-function C ABI.

use pdsim_session::{Command, SessionCore};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

type Queue = Rc<RefCell<VecDeque<Vec<u8>>>>;

/// A session plus its outgoing event queue.
pub struct Instance {
    core: SessionCore,
    queue: Queue,
}

impl Default for Instance {
    fn default() -> Self {
        Self::new()
    }
}

fn push(queue: &Queue, ev: &Value) {
    queue.borrow_mut().push_back(ev.to_string().into_bytes());
}

impl Instance {
    pub fn new() -> Instance {
        let queue: Queue = Rc::default();
        let q = queue.clone();
        let core = SessionCore::new(Box::new(move |ev| push(&q, &ev)));
        Instance { core, queue }
    }

    /// Handles one JSON command. A bad command is an `Err` and also queues an
    /// `error` event.
    pub fn send(&mut self, json: &[u8], now_ms: f64) -> Result<(), String> {
        match serde_json::from_slice::<Command>(json) {
            Ok(c) => {
                self.core.handle(c, now_ms);
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                push(&self.queue, &json!({"type": "error", "message": msg}));
                Err(msg)
            }
        }
    }

    /// Runs due work; returns when to call again (host clock, ms), or `-1.0`
    /// when idle.
    pub fn tick(&mut self, now_ms: f64) -> f64 {
        self.core.tick(now_ms).next_wake_ms.unwrap_or(-1.0)
    }

    /// Returns 0 if no event is queued. Otherwise returns the next event's
    /// JSON length; it is copied into `buf` and popped only if it fits.
    pub fn poll(&mut self, buf: &mut [u8]) -> usize {
        let mut q = self.queue.borrow_mut();
        let Some(front) = q.front() else { return 0 };
        let n = front.len();
        if n <= buf.len() {
            buf[..n].copy_from_slice(front);
            q.pop_front();
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn example() -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/lua-stress")
            .display()
            .to_string()
    }

    fn drain(i: &mut Instance) -> Vec<Value> {
        let mut out = vec![];
        loop {
            let n = i.poll(&mut []);
            if n == 0 {
                return out;
            }
            let mut buf = vec![0; n];
            assert_eq!(i.poll(&mut buf), n);
            out.push(serde_json::from_slice(&buf).unwrap());
        }
    }

    #[test]
    fn instance_runs_lua_stress_in_turbo() {
        let mut i = Instance::new();
        let data = std::env::temp_dir().join(format!("pdsim-wasm-test-{}", std::process::id()));
        let load = json!({"type": "load", "path": example(), "dataDir": data.display().to_string()});
        i.send(load.to_string().as_bytes(), 0.0).unwrap();
        i.send(br#"{"type":"setMode","mode":"turbo"}"#, 0.0).unwrap();
        for k in 0..10 {
            assert!(i.tick(k as f64) >= 0.0);
        }
        let events = drain(&mut i);
        let count = |t: &str| events.iter().filter(|e| e["type"] == t).count();
        assert_eq!(count("loaded"), 1, "{events:?}");
        assert_eq!(count("frame"), 10);
        for f in events.iter().filter(|e| e["type"] == "frame") {
            let fb = base64::engine::general_purpose::STANDARD
                .decode(f["fb"].as_str().unwrap())
                .unwrap();
            assert_eq!(fb.len(), 52 * 240);
        }
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn poll_reports_length_without_popping_when_buffer_is_small() {
        let mut i = Instance::new();
        i.send(br#"{"type":"hello"}"#, 0.0).unwrap();
        let n = i.poll(&mut [0u8; 4]);
        assert!(n > 4);
        let mut buf = vec![0; n];
        assert_eq!(i.poll(&mut buf), n);
        serde_json::from_slice::<Value>(&buf).unwrap();
        // Any further events (hello may emit several) drain to empty.
        drain(&mut i);
        assert_eq!(i.poll(&mut buf), 0);
    }

    #[test]
    fn bad_json_is_an_error_event() {
        let mut i = Instance::new();
        assert!(i.send(b"{nope", 0.0).is_err());
        assert!(i.send(br#"{"type":"bogus"}"#, 0.0).is_err());
        let events = drain(&mut i);
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e["type"] == "error"));
    }

    #[test]
    fn idle_tick_is_minus_one() {
        assert_eq!(Instance::new().tick(5.0), -1.0);
    }
}
