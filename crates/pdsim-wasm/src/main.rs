//! Entry point of the browser build. `main` does nothing; the host calls the
//! exported C functions below. The session is single-threaded, so one
//! thread-local `Instance` is all the state there is.

use pdsim_wasm::Instance;
use std::cell::RefCell;

thread_local! {
    static INSTANCE: RefCell<Instance> = RefCell::new(Instance::new());
}

fn main() {}

/// Resets the session to a fresh one.
#[no_mangle]
pub extern "C" fn pdsim_new() {
    INSTANCE.with(|i| *i.borrow_mut() = Instance::new());
}

/// Sends one JSON command. Returns 0 on success, 1 on error (an `error`
/// event is queued too).
///
/// # Safety
/// `ptr` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn pdsim_send(ptr: *const u8, len: usize, now_ms: f64) -> i32 {
    // SAFETY: the caller guarantees `ptr..ptr+len` is readable.
    let json: &[u8] = if len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len) }
    };
    INSTANCE.with(|i| i.borrow_mut().send(json, now_ms).is_err()) as i32
}

/// Runs due work; returns the next wake time in ms, or -1 when idle.
#[no_mangle]
pub extern "C" fn pdsim_tick(now_ms: f64) -> f64 {
    INSTANCE.with(|i| i.borrow_mut().tick(now_ms))
}

/// Copies the next event into `buf` if it fits. Returns its length, or 0 if
/// none is queued.
///
/// # Safety
/// `buf` must point to `cap` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn pdsim_poll(buf: *mut u8, cap: usize) -> usize {
    // SAFETY: the caller guarantees `buf..buf+cap` is writable.
    let buf: &mut [u8] = if cap == 0 {
        &mut []
    } else {
        unsafe { std::slice::from_raw_parts_mut(buf, cap) }
    };
    INSTANCE.with(|i| i.borrow_mut().poll(buf))
}
