# Lua Games in the Browser (WASM) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run a Lua Playdate game entirely in a browser tab (static site, no `pdsim serve`), with the full perf panel and device timings identical to the native build.

**Architecture:** Build the existing crates for `wasm32-unknown-emscripten`. Emscripten supplies libc, `setjmp`/`longjmp` for the vendored Lua, and an in-memory filesystem, so `std::fs` keeps working. Split `pdsim-session` into a platform-free `SessionCore` driven by `handle(cmd, now_ms)` / `tick(now_ms)`, keep the threaded `Session` as a thin native wrapper, and expose the core to JS through a new `pdsim-wasm` crate (four C ABI functions) running in a Web Worker. The UI gets a third transport (`kind: "wasm"`) and keeps speaking the existing command/event protocol.

**Tech Stack:** Rust (stable), Emscripten (emsdk 4.x), mlua 0.12, Node 22 (headless parity runs), TypeScript 5.9 + Vite 6, vitest 3.2.4, fflate 0.8.2 (zip loading).

**Spec:** [docs/wasm-port.md](../../wasm-port.md), phases 0–2 (plus the Phase 6 "download the report and trace" item, which falls out of Task 4). C games are a separate plan: [2026-11-05-wasm-c-games.md](2026-11-05-wasm-c-games.md).

## Global Constraints

- **Same numbers:** "`report.json` from native must equal `report.json` from WASM (compare cycles exactly, not within a tolerance)."
- **One protocol:** the browser build speaks the existing command/event protocol ([session-protocol.md](../../session-protocol.md)); the UI only gets a new transport. Protocol additions must be backwards compatible (new optional fields, new event types).
- **Native builds keep working** at every commit: `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` pass, and the desktop app and `pdsim serve` don't regress.
- **No SDK files in the bundle or the repo.** CoreLibs, the system font and `C_API/` stay user-supplied. Never commit SDK files.
- Target: `wasm32-unknown-emscripten`. Single-threaded WASM only (no COOP/COEP headers, so GitHub Pages works).
- Workspace `rust-version` stays 1.80 for native builds; the WASM build may require a newer stable, documented in the README.
- Pinned JS versions: `vitest@3.2.4`, `fflate@0.8.2`. Existing pins (`vite 6.4.5`, `typescript 5.9.3`) are unchanged.

## Review Focus

1. **A Rust error or panic raised inside a Lua API binding on Emscripten** (bad argument from game code, inside and outside `pcall`): the game sees a Lua error or the session reports `error`, never a trapped/aborted WASM instance. Test: Task 1, `lua_errors_cross_rust_frames`.
2. **A command arriving while the core waits for the next frame** (button press during device-speed pacing): handled immediately, and the frame stays on its original schedule rather than restarting the wait. Test: Task 3, `command_during_wait_keeps_the_schedule`.
3. **An event larger than the JS poll buffer** (a `flame` or `stats` event bigger than the initial 64 KiB buffer): the caller grows the buffer and gets the whole event; nothing is dropped or truncated. Tests: Task 5, `poll_reports_length_without_popping_when_buffer_is_small`; Task 6, `drainEvents grows the buffer for a large event`.
4. **A dropped folder whose files are not under one top-level folder, or a zip with a wrapping folder or `__MACOSX/` entries:** the game still lands with `main.lua` (or `Source/main.lua`) directly under `/game`. Test: Task 7, `gamePaths` cases.
5. **Reloading the page after the game wrote save data:** `playdate.datastore` data is still there. Test: Task 7, `load injects dataDir under /data`, plus the manual reload check in Task 8.

---

## File Structure

| File | Responsibility |
|---|---|
| `.cargo/config.toml` (create) | Emscripten target flags shared by every WASM build, and `runner = "node"` |
| `scripts/wasm-node.sh` (create) | Runs a cargo command for the Emscripten target with `NODERAWFS`, for Node-hosted CLI and test runs |
| `scripts/wasm-parity.sh` (create) | Builds native and Node CLIs, runs each Lua example, and byte-compares the reports |
| `crates/pdsim-lua/build.rs` (modify) | Emscripten branch for the Lua defines and longjmp flag |
| `crates/pdsim-runner/{Cargo.toml,src/lib.rs}` (modify) | `c-games` feature gating `pdsim-arm`; `RunConfig` unchanged |
| `crates/pdsim-cli/Cargo.toml`, `crates/pdsim-session/Cargo.toml` (modify) | Forward the `c-games` feature |
| `crates/pdsim-session/src/core.rs` (create) | `SessionCore`: everything the session does except threads, channels and `Instant` |
| `crates/pdsim-session/src/lib.rs` (modify) | `Command`, native `Session` wrapper (thread, channel, clock, trace file writing) |
| `crates/pdsim-wasm/` (create) | `Instance` (core + event queue) as a lib, tested natively; `src/main.rs` holds the C ABI exports; `build.rs` adds Emscripten link args |
| `app/src/wasm/protocol.ts` (create) | Message types between page and worker |
| `app/src/wasm/worker.ts` (create) | Loads the module, runs the tick loop, files into MEMFS/IDBFS |
| `app/src/wasm/events.ts` (create) | `drainEvents` (poll loop with buffer growth) and `delayFor` (scheduling), pure and unit-tested |
| `app/src/wasm/files.ts` (create) | Folder/zip/SDK file collection and path mapping, pure and unit-tested |
| `app/src/backend.ts` (modify) | `kind: "wasm"` transport and selection |
| `app/src/main.ts`, `app/index.html` (modify) | Drop zone, "Load SDK folder" button, trace download |
| `app/vite.config.ts`, `app/package.json` (modify) | `build:web` mode, `test` script |
| `.github/workflows/ci.yml` (modify) | `wasm` job: parity check and web bundle |
| `docs/session-protocol.md`, `README.md`, `docs/architecture.md` (modify) | Protocol additions, "Run in the browser" |

---

### Task 1: Emscripten toolchain, `c-games` feature, Node parity

**Files:**
- Create: `.cargo/config.toml`, `scripts/wasm-node.sh`, `scripts/wasm-parity.sh`
- Modify: `crates/pdsim-lua/build.rs`, `crates/pdsim-runner/Cargo.toml`, `crates/pdsim-runner/src/lib.rs`, `crates/pdsim-cli/Cargo.toml`, `crates/pdsim-session/Cargo.toml`
- Test: `crates/pdsim-lua/tests/game.rs`, `crates/pdsim-runner/src/lib.rs` (tests module)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - Cargo feature `c-games` on `pdsim-runner` (default on; enables the optional `pdsim-arm` dependency), forwarded as `c-games = ["pdsim-runner/c-games"]` (default on) by `pdsim-session` and `pdsim-cli`.
  - Without `c-games`, `pdsim_runner::load` on a C game returns `GameError("C games are not supported in this build (built without the c-games feature)")`.
  - `scripts/wasm-node.sh <cargo args…>`: runs cargo with `--target wasm32-unknown-emscripten` and `EMCC_CFLAGS=-sNODERAWFS=1`, so the result runs under Node against the real filesystem.
  - `scripts/wasm-parity.sh`: exit 0 only if every Lua project's native and Node reports are byte-identical.

- [ ] **Step 1: Write the failing tests**

In `crates/pdsim-lua/tests/game.rs`, using the existing `TempGame` helper:

```rust
#[test]
fn lua_errors_cross_rust_frames() {
    let g = TempGame::new("errors", r#"
        local n = 0
        function playdate.update()
            n += 1
            local ok = pcall(playdate.graphics.setColor, "bogus")
            assert(not ok, "bad argument should raise")
            if n == 3 then error("boom") end
        end
    "#);
    let mut game = g.load();
    assert!(game.frame(None).is_ok());
    assert!(game.frame(None).is_ok());
    let err = game.frame(None).unwrap_err();
    assert!(err.0.contains("boom"), "{}", err.0);
}
```

In `crates/pdsim-runner/src/lib.rs` tests:

```rust
#[cfg(not(feature = "c-games"))]
#[test]
fn c_games_need_the_feature() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/c-stress");
    let err = load(&root, &RunConfig::default()).err().unwrap();
    assert!(err.0.contains("c-games feature"), "{}", err.0);
}
```

(`examples/c-stress` is detected as C even without a built `pdex.elf`; if `detect` needs the ELF, point the test at a temp dir containing an empty `pdex.elf`.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pdsim-runner --no-default-features`
Expected: FAIL to compile (no `c-games` feature / `pdsim-arm` not optional).
Run: `cargo test -p pdsim-lua --test game lua_errors_cross_rust_frames`
Expected: PASS natively (it is a guard for the Emscripten run in Step 6; if it fails natively, fix that first).

- [ ] **Step 3: Add the `c-games` feature**

Make `pdsim-arm` optional in `crates/pdsim-runner/Cargo.toml` with `[features] default = ["c-games"]`, `c-games = ["dep:pdsim-arm"]`; put `load_c`'s body behind `#[cfg(feature = "c-games")]` and return the error above otherwise. Forward the feature from `pdsim-session` and `pdsim-cli` (each with `default-features = false` on the `pdsim-runner` dependency and its own `default = ["c-games"]`).

- [ ] **Step 4: Run native tests**

Run: `cargo test -p pdsim-runner --no-default-features && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Emscripten build of the Lua core**

- `crates/pdsim-lua/build.rs`: add a `"emscripten"` arm to the `target_os` match that defines none of `LUA_USE_LINUX`/`LUA_USE_POSIX`/`LUA_USE_MACOSX` (portable defaults), and add the cc flag matching the longjmp mode below.
- `.cargo/config.toml`:

```toml
[target.wasm32-unknown-emscripten]
runner = "node"
rustflags = [
  "-C", "link-arg=-sSUPPORT_LONGJMP=wasm",
  "-C", "link-arg=-sALLOW_MEMORY_GROWTH=1",
  "-C", "link-arg=-sSTACK_SIZE=5MB",
]
```

- Settle the unwinding/longjmp pair here (spec, "Unwinding"). Try `-sSUPPORT_LONGJMP=wasm` with the Rust target's default panic strategy. If emcc refuses to mix the exception models, or Step 6 shows a trap, switch to the JS-based pair: `-sSUPPORT_LONGJMP=emscripten` (in both `build.rs` and `.cargo/config.toml`). If neither works, use `panic = "abort"` for the Emscripten target only. Record the choice and the reason in a comment at the top of `.cargo/config.toml`.
- `scripts/wasm-node.sh`: `set -euo pipefail; EMCC_CFLAGS="-sNODERAWFS=1" exec cargo "$@" --target wasm32-unknown-emscripten`.

- [ ] **Step 6: Run the Lua tests under Node**

Run: `scripts/wasm-node.sh test -p pdsim-lua`
Expected: PASS, including `lua_errors_cross_rust_frames` (no `RuntimeError: unreachable` / abort).

- [ ] **Step 7: Write the parity script**

`scripts/wasm-parity.sh`: build `cargo build --release -p pdsim-cli` and `scripts/wasm-node.sh build --release -p pdsim-cli --no-default-features`; for each project in `examples/lua-stress` and `calibration/bench-lua`, run both with `run <project> --frames 300 -q --report <tmp>/{native,wasm}.json` (Node: `node target/wasm32-unknown-emscripten/release/pdsim.js run …`), then `cmp` the two reports and print the first differing line with `diff` on failure.

- [ ] **Step 8: Run the parity check**

Run: `scripts/wasm-parity.sh`
Expected: `parity OK: 2 projects`. If reports differ in cycle counts, do Task 2; otherwise skip Task 2 and note "libm parity confirmed" in the commit message.

- [ ] **Step 9: Commit**

```bash
git add .cargo/config.toml scripts/wasm-node.sh scripts/wasm-parity.sh crates/pdsim-lua crates/pdsim-runner crates/pdsim-cli/Cargo.toml crates/pdsim-session/Cargo.toml
git commit -m "Build the Lua runner for Emscripten and check report parity under Node"
```

---

### Task 2: Deterministic libm (only if Task 1, Step 8 reports a cycle difference)

**Files:**
- Create: `crates/pdsim-lua/src/libm_shim.rs`, `crates/pdsim-lua/vendor/pdsim_libm.h`
- Modify: `crates/pdsim-lua/build.rs`, `crates/pdsim-lua/Cargo.toml` (add `libm = "0.2"`), `crates/pdsim-lua/src/lib.rs`
- Test: `crates/pdsim-lua/tests/game.rs`

**Interfaces:**
- Consumes: Task 1's parity script.
- Produces: `#[no_mangle] extern "C" fn pdsim_sin(f64) -> f64` (and `cos tan asin acos atan atan2 exp log log2 log10 pow fmod sqrt floor ceil`), each a direct call into the `libm` crate, used by `lmathlib.c` on **every** target.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn math_uses_the_portable_libm() {
    // Values chosen where glibc and musl are known to round differently in the last ULP.
    let g = TempGame::new("libm", r#"
        out = string.format("%a %a %a", math.sin(1e22), math.exp(709.7), (2.0)^0.5)
        function playdate.update() end
    "#);
    let game = g.load();
    assert_eq!(game.global_string("out"), format!("{} {} {}", hex(libm::sin(1e22)), hex(libm::exp(709.7)), hex(libm::pow(2.0, 0.5))));
}
```

(`global_string` is a test-only accessor; add it to `LuaGame` under `#[doc(hidden)]` if no equivalent exists. `hex` formats an `f64` the way Lua's `%a` does.)

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p pdsim-lua --test game math_uses_the_portable_libm`
Expected: FAIL (glibc result differs, or the accessor is missing).

- [ ] **Step 3: Route `lmathlib.c` through the shim**

`vendor/pdsim_libm.h` declares the `pdsim_*` functions and `#define`s `l_mathop(sin)` … to them; `build.rs` force-includes it for `lmathlib.c` only (`-include pdsim_libm.h`). Do not edit `lmathlib.c` itself.

- [ ] **Step 4: Run tests and parity**

Run: `cargo test -p pdsim-lua && scripts/wasm-node.sh test -p pdsim-lua && scripts/wasm-parity.sh`
Expected: PASS, `parity OK: 2 projects`.

- [ ] **Step 5: Commit**

```bash
git add crates/pdsim-lua
git commit -m "Use the libm crate for Lua math on every target so WASM and native agree"
```

---

### Task 3: `SessionCore`, a steppable session

**Files:**
- Create: `crates/pdsim-session/src/core.rs`
- Modify: `crates/pdsim-session/src/lib.rs`
- Test: `crates/pdsim-session/src/core.rs` (tests module); existing tests in `lib.rs` stay unchanged

**Interfaces:**
- Consumes: `pdsim_runner::load`, `Command` (unchanged in this task).
- Produces (`pub use core::{SessionCore, Tick}` from `pdsim_session`):
  - `SessionCore::new(emit: Box<dyn Fn(Value)>) -> SessionCore` (no `Send` bound; the native wrapper builds it on its thread).
  - `SessionCore::handle(&mut self, c: Command, now_ms: f64)`. `Quit` drops the game and stops running.
  - `SessionCore::tick(&mut self, now_ms: f64) -> Tick`: runs **at most one** frame, when running, loaded and due (`turbo`, or `now_ms >= next_frame_ms`).
  - `pub struct Tick { pub next_wake_ms: Option<f64> }`. `None` means idle until the next command; in turbo it is `Some(now_ms)`.
  - Pacing values move unchanged from `Runner::pace`: the target is `anchor_wall_ms + (device_ms - anchor_device_ms) / speed`; if `now_ms - target > 250.0`, re-anchor to `now_ms`. Stats flush when `now_ms - last_stats_ms >= 100.0` (`STATS_INTERVAL`).
  - Native `Session` keeps its public API (`spawn`, `send`, `send_json`, `Drop`). Its thread computes `now_ms` from one `Instant` taken at spawn, waits with `recv_timeout(next_wake - now)` (200 ms when idle), calls `handle` for each command and `tick` on timeout.

- [ ] **Step 1: Write the failing tests** (in `core.rs`, no threads, no sleeping)

```rust
fn core() -> (SessionCore, Rc<RefCell<Vec<Value>>>)            // collects emitted events
fn frames(ev: &[Value]) -> usize                               // count of type == "frame"
fn load_example(c: &mut SessionCore)                           // Load examples/lua-stress at now_ms = 0.0

#[test] fn idle_core_never_wakes()
    // new core: tick(0.0).next_wake_ms == None and no events

#[test] fn device_mode_runs_one_frame_per_due_tick()
    // after load_example: t = tick(0.0) -> 1 frame, w = t.next_wake_ms.unwrap(), w > 0.0
    // tick(w - 1.0) -> still 1 frame, returns Some(w)
    // tick(w) -> 2 frames

#[test] fn command_during_wait_keeps_the_schedule()
    // after one tick: w = next wake; handle(Button{name:"a", down:true}, w / 2.0)
    // tick(w / 2.0).next_wake_ms == Some(w) and frame count unchanged

#[test] fn host_falling_behind_reanchors()
    // after one tick at 0.0 with wake w: tick(w + 1000.0) runs a frame and returns a wake >= w + 1000.0

#[test] fn turbo_wakes_immediately()
    // handle(SetMode{mode:"turbo"}, 0.0); tick(5.0).next_wake_ms == Some(5.0)

#[test] fn pause_goes_idle_and_step_runs_exactly_one_frame()
    // handle(Pause, 0.0) -> tick(...) None; handle(Step, 0.0) emits exactly one more frame
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p pdsim-session core::`
Expected: FAIL to compile (`SessionCore` not defined).

- [ ] **Step 3: Move `Runner` into `SessionCore`**

Move the `Runner` fields and methods into `core.rs` as `SessionCore`, replacing `Instant` fields with `f64` milliseconds supplied by callers (`anchor_wall_ms`, `last_stats_ms`) and the blocking `pace` with the `tick` rules above. Rewrite `Session::spawn`'s thread loop as described in Interfaces. `Step` keeps its current behaviour (one frame, stays paused), done inside `handle`.

- [ ] **Step 4: Run all session tests**

Run: `cargo test -p pdsim-session`
Expected: PASS, including the existing `device_speed_runs_in_real_time_and_turbo_is_faster` and `breaks_on_over_budget_frame_and_steps` unchanged.

- [ ] **Step 5: Check the native hosts**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`, then `cargo run -p pdsim-cli -- serve examples/lua-stress` with `cd app && npm run dev` and open `http://127.0.0.1:1420/`.
Expected: the game plays at device speed, turbo is faster, pause/step/break-on-over-budget work as before.

- [ ] **Step 6: Commit**

```bash
git add crates/pdsim-session
git commit -m "Split the session into a steppable SessionCore and a thin threaded wrapper"
```

---

### Task 4: Protocol additions: `traceData` event, `dataDir`/`sdk` on `load`

**Files:**
- Modify: `crates/pdsim-session/src/lib.rs`, `crates/pdsim-session/src/core.rs`, `docs/session-protocol.md`
- Test: `crates/pdsim-session/src/core.rs`, `crates/pdsim-session/src/lib.rs` (tests modules)

**Interfaces:**
- Consumes: `SessionCore` (Task 3).
- Produces:
  - `Command::Load { path, calibration, #[serde(default)] data_dir: Option<String>, #[serde(default)] sdk: Option<String> }` (JSON `dataDir`, `sdk`). They fill `RunConfig::data_dir` / `RunConfig::sdk` when present; absent means today's defaults.
  - The core no longer touches the filesystem for traces. `ExportTrace{path}` emits `{"type":"traceData","path":<path or null>,"frame":<frame index>,"frames":<n>,"trace":<chrome trace JSON value>}`.
  - The native `Session` wrapper intercepts `traceData` in its emit closure: it writes the file exactly as today (same default path `$HOME/pdsim-trace-{frame}.json`) and emits the existing `traceExported{path, frames}` or `error`. Native UIs see no change.
  - Browser hosts receive `traceData` and save it as a download (Task 8).

- [ ] **Step 1: Write the failing tests**

```rust
// core.rs
#[test] fn export_trace_emits_trace_data_without_writing_files()
    // after load + 3 ticks in turbo: handle(ExportTrace{path: None}, t)
    // last event: type == "traceData", frames == 3, trace["traceEvents"] is a non-empty array, path is null
#[test] fn load_uses_the_given_data_dir()
    // Load{dataDir: Some(tmp/"saves")} of examples/lua-stress -> tmp/"saves" exists afterwards

// lib.rs
#[test] fn native_session_still_writes_trace_files()
    // spawn, load lua-stress, wait for a frame, send ExportTrace{path: Some(tmp/"t.json")}
    // wait_for("traceExported") has path == tmp/"t.json" and the file parses as JSON
#[test] fn load_json_without_new_fields_still_parses()
    // serde_json::from_value::<Command>(json!({"type":"load","path":"x"})) is Ok
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p pdsim-session`
Expected: FAIL (no `data_dir` field; `traceData` not emitted).

- [ ] **Step 3: Implement**

Add the fields and the emit-closure interception described above. Document `dataDir`, `sdk` and `traceData` in `docs/session-protocol.md` (`traceData` noted as "sent by in-process hosts that have no filesystem; native hosts convert it to `traceExported`").

- [ ] **Step 4: Run tests**

Run: `cargo test -p pdsim-session && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/pdsim-session docs/session-protocol.md
git commit -m "Let hosts pick the data dir and SDK, and return traces as data"
```

---

### Task 5: `pdsim-wasm` crate

**Files:**
- Create: `crates/pdsim-wasm/Cargo.toml`, `crates/pdsim-wasm/build.rs`, `crates/pdsim-wasm/src/lib.rs`, `crates/pdsim-wasm/src/main.rs`, `scripts/wasm-web-smoke.mjs`
- Modify: `Cargo.toml` (workspace members)
- Test: `crates/pdsim-wasm/src/lib.rs` (tests module), `scripts/wasm-web-smoke.mjs`

**Interfaces:**
- Consumes: `SessionCore::{new, handle, tick}`, `Tick` (Task 3), `Command::Load{dataDir, sdk}` (Task 4).
- Produces:
  - Lib `pdsim_wasm`: `pub struct Instance`; `Instance::new() -> Instance`; `send(&mut self, json: &[u8], now_ms: f64) -> Result<(), String>` (bad JSON or unknown command → `Err` *and* an `error` event queued); `tick(&mut self, now_ms: f64) -> f64` (`-1.0` for idle, otherwise `next_wake_ms`); `poll(&mut self, buf: &mut [u8]) -> usize`: 0 if the queue is empty; otherwise the byte length of the next event's JSON, copied and popped only if it fits in `buf` (if it doesn't fit, return the length and keep the event).
  - Bin `pdsim-web` (`src/main.rs`, empty `fn main` and one `thread_local!` `Instance`) with C ABI exports: `pdsim_new()`, `pdsim_send(ptr: *const u8, len: usize, now_ms: f64) -> i32` (0 ok, 1 error), `pdsim_tick(now_ms: f64) -> f64`, `pdsim_poll(buf: *mut u8, cap: usize) -> usize`.
  - Build output: `target/wasm32-unknown-emscripten/release/pdsim-web.{js,wasm}`, an ES6 module factory (`-sMODULARIZE -sEXPORT_ES6 -sEXPORT_NAME=createPdsim`) with `FS` and `HEAPU8` exported, IDBFS linked, `-sENVIRONMENT=worker,node`, `-sEXPORTED_FUNCTIONS=_pdsim_new,_pdsim_send,_pdsim_tick,_pdsim_poll,_malloc,_free`. `build.rs` emits these with `cargo:rustc-link-arg-bin=pdsim-web=…` only when `CARGO_CFG_TARGET_OS == "emscripten"`. The crate depends on `pdsim-session` with `default-features = false` (no C games yet).

- [ ] **Step 1: Write the failing tests** (native, `lib.rs`)

```rust
#[test] fn instance_runs_lua_stress_in_turbo()
    // send load(examples/lua-stress, dataDir tmp) and setMode turbo; tick 10 times;
    // drain poll into Vec<Value>: exactly one "loaded", 10 "frame" events, each fb decodes to 52*240 bytes
#[test] fn poll_reports_length_without_popping_when_buffer_is_small()
    // after hello: n = poll(&mut [0u8; 4]) > 4; poll(&mut vec![0; n]) == n and parses as JSON; then poll(..) == 0
#[test] fn bad_json_is_an_error_event()
    // send(b"{nope") is Err, next polled event has type "error"
#[test] fn idle_tick_is_minus_one()
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p pdsim-wasm`
Expected: FAIL to compile (crate doesn't exist).

- [ ] **Step 3: Implement the crate** as specified in Interfaces. The event queue is `Rc<RefCell<VecDeque<Vec<u8>>>>` shared with the core's emit closure (events serialized once, on emit).

- [ ] **Step 4: Run native tests**

Run: `cargo test -p pdsim-wasm && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Write the Node smoke test**

`scripts/wasm-web-smoke.mjs`: import the built factory, `FS.mkdirTree('/game')`, copy `examples/lua-stress` into it with `FS.writeFile`, then `pdsim_send` load `/game` (dataDir `/data/lua-stress`) and turbo, tick 60 times, poll everything. Exit 1 unless there is one `loaded`, 60 `frame`s and no `error`.

- [ ] **Step 6: Build and run it**

Run: `cargo build --release -p pdsim-wasm --target wasm32-unknown-emscripten --bin pdsim-web && node scripts/wasm-web-smoke.mjs`
Expected: `smoke OK: 60 frames`.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/pdsim-wasm scripts/wasm-web-smoke.mjs
git commit -m "Add pdsim-wasm: the session behind a four-function C ABI for the browser"
```

---

### Task 6: Worker and the `wasm` transport

**Files:**
- Create: `app/src/wasm/protocol.ts`, `app/src/wasm/events.ts`, `app/src/wasm/worker.ts`, `app/src/wasm/events.test.ts`
- Modify: `app/src/backend.ts`, `app/package.json` (devDependency `vitest@3.2.4`, script `"test": "vitest run"`)

**Interfaces:**
- Consumes: the four exports and `createPdsim` factory (Task 5).
- Produces:
  - `protocol.ts`: `type ToWorker = { kind: "cmd"; cmd: Command } | { kind: "files"; root: "/game" | "/sdk"; files: { path: string; data: ArrayBuffer }[] }` and `type FromWorker = { kind: "ready" } | { kind: "event"; event: Event } | { kind: "fatal"; message: string }`.
  - `events.ts`: `drainEvents(poll: (ptr: number, cap: number) => number, heap: () => Uint8Array, buf: { ptr: number; cap: number }, alloc: (n: number) => number, free: (p: number) => void): Event[]`, which loops `poll` until it returns 0 and reallocates `buf` to the next power of two ≥ the reported length when it doesn't fit; and `delayFor(nextWakeMs: number, nowMs: number): number | null` (`null` for `-1`, otherwise `max(0, nextWakeMs - nowMs)`).
  - `worker.ts` tick loop: on each `cmd`, call `pdsim_send`, cancel the pending timer and pump now. `pump` calls `pdsim_tick(performance.now())` repeatedly while the delay is 0, up to an 8 ms slice, posts drained events, then schedules the next pump with `setTimeout(delay)` (or a `MessageChannel` post when the slice ran out with delay 0). Initial poll buffer 64 KiB.
  - `Backend.kind` becomes `"tauri" | "websocket" | "wasm"`; `Backend` gains optional `loadFiles?(root: "/game" | "/sdk", files: { path: string; data: ArrayBuffer }[]): void`. `connect()` picks `wasm` when not in Tauri and (`?backend=wasm` or no `?ws=`); `?backend=ws` forces the WebSocket.

- [ ] **Step 1: Write the failing tests** (`events.test.ts`, with a fake `poll` backed by a JS array of encoded events and a fake heap)

```ts
test("drainEvents returns all queued events in order", ...)        // 3 small events -> 3 parsed, in order
test("drainEvents grows the buffer for a large event", ...)        // one 200 KiB event, buf.cap 64 KiB -> event intact, buf.cap === 262144, old ptr freed
test("delayFor is null when idle and never negative", ...)         // delayFor(-1, 5) === null; delayFor(3, 5) === 0; delayFor(40, 5) === 35
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd app && npm install && npm test`
Expected: FAIL (module `./events` not found).

- [ ] **Step 3: Implement `protocol.ts`, `events.ts`, `worker.ts` and the backend change** as specified. The worker is created with `new Worker(new URL("./wasm/worker.ts", import.meta.url), { type: "module" })` and loads `pdsim-web.js` from the site root (`new URL("/pdsim-web.js", location.origin)`, overridable with `?wasm=`).

- [ ] **Step 4: Run tests and type-check**

Run: `cd app && npm test && npx tsc --noEmit`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/wasm app/src/backend.ts app/package.json app/package-lock.json
git commit -m "Add the in-browser wasm transport: a worker driving pdsim-web"
```

---

### Task 7: Loading games, save data and the SDK in the browser

**Files:**
- Create: `app/src/wasm/files.ts`, `app/src/wasm/files.test.ts`
- Modify: `app/src/wasm/worker.ts`, `app/src/main.ts`, `app/index.html`, `app/package.json` (dependency `fflate@0.8.2`)

**Interfaces:**
- Consumes: `Backend.loadFiles`, `ToWorker`/`FromWorker` (Task 6); `Command::Load{dataDir, sdk}` (Task 4).
- Produces:
  - `gamePaths(entries: { path: string }[]): Map<string, string>` maps each input path to its path under `/game`, stripping a single shared top-level folder and dropping `__MACOSX/`, `.DS_Store` and dotfiles.
  - `gameName(entries: { path: string }[]): string` is the stripped top-level folder name, or `"game"`.
  - `sdkPaths(entries)`: like `gamePaths` for `/sdk`, but keeps only `CoreLibs/**`, `Resources/Fonts/**` and `C_API/**`, relative to the folder that contains `C_API`.
  - `unzipEntries(data: Uint8Array): { path: string; data: ArrayBuffer }[]` (fflate `unzipSync`).
  - Worker behaviour:
    - On `files` for `/game`, it wipes and rewrites `/game`.
    - On a `load` cmd, it injects `dataDir: "/data/" + gameName` and, if `/sdk/C_API` exists, `sdk: "/sdk"`.
    - At startup it mounts IDBFS at `/data` and `/sdk`, then runs `syncfs(true)` before posting `ready`.
    - It runs `syncfs(false)` at most once per second while running and on `pause`, and stores `/sdk` after an SDK load.
  - UI:
    - In wasm mode the path field is replaced by a drop zone plus an "Open folder…" button (`<input type="file" webkitdirectory>`); `.zip` files are accepted too.
    - A "Load Playdate SDK folder…" button (wasm mode only) shows "SDK: loaded" or "SDK: none (using built-in CoreLibs stand-ins)".
    - After the files are sent, the UI sends the existing `load` command with `path: "/game"`.

- [ ] **Step 1: Write the failing tests** (`files.test.ts`)

```ts
test("gamePaths strips one shared top folder", ...)   // ["mygame/main.lua","mygame/img/a.png"] -> "/game/main.lua","/game/img/a.png"
test("gamePaths keeps Source/ layout", ...)           // ["mygame/Source/main.lua"] -> "/game/Source/main.lua"
test("gamePaths with no shared folder", ...)          // ["main.lua","b.lua"] -> "/game/main.lua","/game/b.lua"
test("gamePaths drops zip and OS junk", ...)          // "__MACOSX/x", "mygame/.DS_Store" absent from the map
test("gameName", ...)                                 // "mygame"; "game" for no shared folder
test("sdkPaths keeps only the three SDK subtrees", ...) // "PlaydateSDK/CoreLibs/graphics.lua" -> "/sdk/CoreLibs/graphics.lua"; "PlaydateSDK/bin/pdc" dropped
test("load injects dataDir under /data", ...)         // pure helper withHostPaths(cmd, "mygame", true) -> { ...cmd, dataDir: "/data/mygame", sdk: "/sdk" }
```

(`withHostPaths(cmd: Command, gameName: string, haveSdk: boolean): Command` lives in `files.ts` so the worker's injection is unit-tested.)

- [ ] **Step 2: Run them to verify they fail**

Run: `cd app && npm test`
Expected: FAIL (module `./files` not found).

- [ ] **Step 3: Implement `files.ts`, the worker FS handling and the UI controls** as specified. Read dropped folders with `DataTransferItem.webkitGetAsEntry()` (recursive `readEntries` until it returns an empty batch).

- [ ] **Step 4: Run tests and type-check**

Run: `cd app && npm test && npx tsc --noEmit`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src app/index.html app/package.json app/package-lock.json
git commit -m "Load games and the SDK into the in-browser filesystem, persist save data"
```

---

### Task 8: `build:web`, trace download, CI and docs

**Files:**
- Modify: `app/vite.config.ts`, `app/package.json`, `app/src/main.ts`, `.github/workflows/ci.yml`, `README.md`, `docs/architecture.md`
- Test: `app/src/wasm/files.test.ts` (one more case), manual browser check

**Interfaces:**
- Consumes: everything above.
- Produces:
  - `npm run build:web` runs `cargo build --release -p pdsim-wasm --target wasm32-unknown-emscripten --bin pdsim-web`, then `vite build --mode web`. Web mode marks `@tauri-apps/*` as external and never imports them (the Tauri branch in `connect()` is a dynamic import, so it stays unloaded). It copies `pdsim-web.{js,wasm}` into `dist/` with relative URLs (`base: "./"`) so the site works from a GitHub Pages subpath.
  - `traceFileName(frame: number): string` returns `pdsim-trace-${frame}.json`. On `traceData`, the UI downloads `JSON.stringify(trace)` as a Blob with that name and shows the existing toast.
  - In wasm mode the UI shows "host slower than device" in the status bar while the stats show device time falling behind real time (re-anchoring keeps happening).
  - CI job `wasm` on ubuntu-24.04: set up emsdk (`mymindstorm/setup-emsdk@v14`, pinned emsdk version), `rustup target add wasm32-unknown-emscripten`, Node 22. Then run `scripts/wasm-parity.sh`, `node scripts/wasm-web-smoke.mjs`, `cd app && npm ci && npm test && npm run build:web`, and print the gzipped `.wasm` size.

- [ ] **Step 1: Write the failing test**

```ts
test("traceFileName", () => expect(traceFileName(130)).toBe("pdsim-trace-130.json"));
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd app && npm test`
Expected: FAIL (`traceFileName` not exported).

- [ ] **Step 3: Implement** the build mode, the download and the status message, the CI job, and the docs: a "Run in the browser" section in `README.md` (build command, `npx vite preview` to try it, Lua-only for now, SDK optional) and a paragraph in `docs/architecture.md` on `SessionCore` and `pdsim-wasm`.

- [ ] **Step 4: Run tests and the build**

Run: `cd app && npm test && npm run build:web && ls dist/pdsim-web.wasm`
Expected: PASS and the file exists. Also `cargo test --workspace` still passes.

- [ ] **Step 5: Manual acceptance (spec "Done when")**

Run `cd app && npx vite preview --outDir dist`, then in Chrome, Firefox and Safari:
- Drop `examples/lua-stress`.
- Check that it plays, the perf panel updates, device-speed and turbo modes work, frame step works, and break on over-budget stops at the same frame index as natively (59 with default calibration).
- Check that the trace downloads and opens in Perfetto.
- Reload the page and check that save data persists.

In turbo mode, compare the stats panel's frame-60 work time with `pdsim run examples/lua-stress --frames 61 --frames-out f.jsonl`; they must be equal.

- [ ] **Step 6: Commit**

```bash
git add app .github/workflows/ci.yml README.md docs/architecture.md
git commit -m "Build the web bundle, download traces in the browser, and check WASM parity in CI"
```
