# Plan: run the simulator in a browser (WebAssembly)

## Goal
Run a Playdate game, Lua or C, entirely in a browser tab: no install, no
`pdsim serve`. Keep the full perf panel and identical device timings. The
page should be a static site (GitHub Pages or similar) that users drop their
game folder onto.

Today a browser can already show the UI. [`app/src/backend.ts`](../app/src/backend.ts)
falls back to a WebSocket to `pdsim serve`, but the simulation itself still
runs natively. This plan moves the simulation into the page.

## Ground rules
- **Same numbers.** The simulator never reads the host clock for costing, so a
  WASM build must give the *same* cycle counts, frame times and hotspots as
  the native build for the same game and input script. That is the main
  acceptance test for every phase: `report.json` from native must equal
  `report.json` from WASM (compare cycles exactly, not within a tolerance).
- **One protocol.** The browser build speaks the existing command/event
  protocol ([session-protocol.md](session-protocol.md)), so the UI only gets a
  new transport, not a new code path.
- **Native builds keep working** at each step, and the desktop app and CLI
  don't regress.
- **No SDK files in the bundle.** CoreLibs, the system font and `pd_api.h`
  stay user-supplied, just as natively (see "SDK files" below).

## Where things stand

| Crate | Browser blockers |
|---|---|
| `pdsim-core`, `pdsim-gfx` | None: pure Rust (serde, png, base64). |
| `pdsim-device` | `std::fs` in `fs.rs`. Fine on Emscripten (MEMFS); needs a trait on `wasm32-unknown-unknown`. |
| `pdsim-lua` | The vendored, patched Lua is C, built by `build.rs` with `cc`. Lua raises errors with `setjmp`/`longjmp`, which `wasm32-unknown-unknown` has no libc for. `sys.rs` also calls `std::fs`. |
| `pdsim-arm` | **Unicorn** (QEMU TCG JIT, emits host machine code, has no WASM backend). Capstone (plain C, compiles fine). The `PlaydateAPI` layout is parsed from the SDK's `pd_api.h` at load time. |
| `pdsim-runner` | Always links `pdsim-arm`. `std::fs::create_dir_all` for the data dir. |
| `pdsim-session` | A `std::thread` loop. Pacing uses `Instant` plus `recv_timeout`/`sleep`, and `std::fs::write` for trace export. |
| `app/` (Vite + TS) | Already runs in a plain browser. Needs a third transport. |

## Target choice: `wasm32-unknown-emscripten`
Emscripten is the path of least resistance:
- It supplies libc and C `setjmp`/`longjmp` (with `-sSUPPORT_LONGJMP=wasm`), so
  the patched Lua builds with almost no change. mlua supports this target.
- `std::fs` works against Emscripten's in-memory FS (MEMFS), and IDBFS can
  persist the game's data directory. So `fs.rs` and `sys.rs` can stay as they
  are at first.
- `std::time::Instant` works (`clock_gettime`).

`wasm32-unknown-unknown` with `wasm-bindgen` would give a smaller,
friendlier JS boundary, but it needs a libc shim for Lua and a filesystem
trait first. We can revisit once everything works.

Note that Rust panics crossing mlua's `catch_unwind` need unwinding support on
Emscripten (WASM exceptions). Phase 0 checks this before anything else depends
on it; if it fails, the fallback is `panic = "abort"` plus making sure no API
binding panics on bad game input.

---

## Phase 0: spike (Lua, headless, Node)
Prove the toolchain before refactoring anything.
1. Install emsdk, then `rustup target add wasm32-unknown-emscripten`.
2. In `crates/pdsim-lua/build.rs`, handle `target_os = "emscripten"`: define
   neither `LUA_USE_LINUX` nor `LUA_USE_POSIX` (no `popen`/`dlopen`), just the
   portable defaults.
3. Add a cargo feature `c-games` (default on) to `pdsim-runner` that gates
   `pdsim-arm`. Build `pdsim-cli --no-default-features` for Emscripten and run
   it under Node with `examples/lua-stress` mounted via `NODEFS`.
4. **Done when** `pdsim run examples/lua-stress --frames 300 --report` under
   Node gives a report identical to native.

Risks to settle here:
- **libm differences.** Emscripten's musl-based `sin`/`cos`/`pow`/`fmod`/`strtod`
  and `%.14g` formatting can differ from glibc in the last ULP. If a game
  branches on such a value, its execution (and so its cycle count) can drift.
  If the reports differ, build a small deterministic libm (for example the
  `libm` crate, exposed to Lua's `lmathlib.c`) and use it natively too, so
  both targets agree.
- **Unwinding** (above).

## Phase 1: make the session steppable
The session is a thread that blocks on a channel and sleeps for pacing.
Split it so a browser event loop can drive it:
- `SessionCore` (new, platform-free): `handle(Command)`,
  `tick(now_ms: f64) -> Tick { next_wake_ms, ... }` and an `emit` callback.
  It holds everything `Session` holds now, except the thread, the channel and
  `Instant`. The wall-clock anchor becomes a plain `f64` milliseconds value
  passed in by the caller.
- Native `Session` becomes a thin wrapper: thread, `Receiver`,
  `recv_timeout(next_wake)` and `Instant`, all calling `SessionCore`.
  Behaviour stays the same.
- Trace export: replace `std::fs::write(path)` with an event carrying the
  trace JSON (or have the core return the bytes and let the native wrapper
  write them). The browser saves it as a download.
- **Done when** the desktop app and `pdsim serve` behave the same and the
  existing session tests pass.

## Phase 2: Lua games in the browser
1. **New crate `crates/pdsim-wasm`** (Emscripten target, `cdylib` or `bin`
   built with `-sMODULARIZE -sEXPORT_ES6`). It exports a small C ABI:
   - `pdsim_new()`, `pdsim_send(json_ptr, len)`, `pdsim_tick(now_ms) -> next_wake_ms`
   - an emit callback into JS (`EM_JS` or a polled event queue,
     `pdsim_poll(buf, cap) -> len`; a polled queue is simpler and avoids
     re-entrancy).
2. **Worker.** `app/src/wasm/worker.ts` loads the module and runs the tick loop
   on `setTimeout`/`MessageChannel` timed to `next_wake_ms`. It forwards
   commands in and events out over `postMessage`. Frame bitmaps go out as
   transferable `ArrayBuffer`s instead of base64 where the protocol allows.
   Keep the JSON shape the same and add a binary side channel only if
   profiling shows base64 costs too much.
3. **Transport.** `backend.ts` gains `kind: "wasm"`, chosen when the page
   isn't in Tauri and no `?ws=` is given (or with `?backend=wasm`).
4. **Loading a game.** Drop a folder (`<input webkitdirectory>` or
   `DataTransferItem.webkitGetAsEntry`) or a `.zip`. JS writes the files into
   MEMFS under `/game` and sends the existing `load` command with that path.
   Mount the data dir as IDBFS at `/data` and `syncfs` after each frame
   batch, so save data survives reloads.
5. **SDK files (optional).** "Load Playdate SDK folder" copies only
   `CoreLibs/`, the system font and `C_API/` into MEMFS `/sdk` and caches them
   in IndexedDB. Without it, Lua games use the bundled CoreLibs stand-ins
   (`pdsim-lua/src/corelibs.rs`), as natively.
6. **Build.** Add `npm run build:web`, a Vite mode that excludes
   `@tauri-apps/*` and copies `pdsim_wasm.{js,wasm}` into `dist/`. No
   COOP/COEP headers are needed (single-threaded WASM), so GitHub Pages
   works.
7. **CI.** Build the web bundle, and run the Phase 0 Node parity check against
   `examples/lua-stress` and any other Lua example.

**Done when** `examples/lua-stress` plays in Chrome, Firefox and Safari with
the perf panel, device-speed and turbo modes, frame stepping, break on
over-budget and trace download, and its numbers match native.

## Phase 3: put a CPU trait in front of Unicorn
Today `emu.rs`, `api.rs` and `abi.rs` take `Unicorn<'_, ArmState>` directly.
The surface they use is small:
- `mem_map`, `mem_read`, `mem_write`
- `reg_read` / `reg_write` (R0–R15, SP, LR, PC, S0–S15)
- `emu_start(begin, until)` / `emu_stop`
- a **block hook** over the code range (cost a block on first entry, charge
  its cached cost each time it runs)
- a **code hook** over the trampoline range (dispatch to Rust API functions)
- `ctl_set_cpu_model(CORTEX_M7)`

Define a `trait Cpu` in `pdsim-arm` with these operations, plus two callbacks:
`on_block(addr, size)` and `on_trampoline(addr)`. Implement it for Unicorn
behind a `unicorn` feature (default on natively). Change `ArmState` access
from `uc.get_data()` to a state the trait passes to the callbacks.

**Done when** the native C path runs only through the trait and
`examples/c-stress` gives the same report as before.

## Phase 4: a Thumb-2 interpreter (`pdsim-thumb`)
A pure-Rust Cortex-M7 interpreter that implements `Cpu`. It replaces Unicorn
in the browser and can optionally replace it natively too (simpler builds, no
C dependency).

**Scope**, only what a `pdex.elf` runs in user code:
- ARMv7E-M Thumb and Thumb-2: data processing, shifts, multiply and divide
  (`MUL`, `MLA`, `MLS`, `UMULL`, `SMULL`, `UMLAL`, `SMLAL`, `SDIV`, `UDIV`), loads and
  stores in every addressing mode, `LDM`/`STM`/`PUSH`/`POP`, `LDRD`/`STRD`,
  exclusives (`LDREX`/`STREX`, trivially), branches including `CBZ`/`CBNZ`,
  `TBB`/`TBH` and `BLX reg`, `IT` blocks, bitfield ops (`BFI`, `UBFX`, `SBFX`,
  `CLZ`, `RBIT`, `REV*`), and the DSP extension (`SMLA<x><y>`, `QADD`, `SSAT`,
  `USAT`, the `UADD8`-style SIMD ops, `SEL`).
- FPv5: single precision for certain; double precision only if the device
  toolchain emits it. Check with `arm-none-eabi-objdump` across the example
  and calibration ELFs, and use softfloat otherwise. This means `VLDR`/`VSTR`,
  `VPUSH`/`VPOP`, `VMOV` variants, arithmetic, `VFMA`, `VSQRT`, `VCVT`,
  `VCMP` + `VMRS APSR_nzcv`, and the FPSCR rounding mode (round-to-nearest,
  flush-to-zero as configured). IEEE results must be bit-exact, which Rust
  `f32` ops are.
- No interrupts, MPU, privilege levels or system registers beyond what
  startup touches. Unknown instructions fail with the same crash message
  format as `crash_message` today.

**Design:**
- Decode once per basic block into a compact micro-op vector, cached by
  address (the same cache key the block hook already uses). Run blocks in a
  loop: `on_block` at block entry, then the micro-ops. Code is mapped
  read-only after load, so no invalidation is needed. Add a guard anyway.
- **Block boundaries must match** what Unicorn/Capstone produce today, so
  that per-block costs, and so cycle totals, stay identical. Keep Capstone
  for costing at first (it builds for Emscripten). Later the interpreter's
  decoder can classify instructions itself and Capstone can go.
- Flat memory: a few `Vec<u8>` regions (code, heap, stack, API, trampolines)
  with bounds-checked little-endian access. An unmapped access is a crash, as
  in Unicorn.

**Testing:**
- **Differential, lockstep against Unicorn natively.** Run the same ELF on
  both, compare registers and the memory write log at each block boundary,
  and report the first divergence with disassembly. Run this on
  `examples/c-stress`, the calibration benchmark projects, and a corpus of
  small generated test functions (random instruction streams per encoding,
  for flag-setting edge cases).
- Per-instruction unit tests for the awkward cases: `IT` with flag-setting,
  `ADC`/`SBC` carry, shifted-register carry-out, `SSAT`/`USAT` Q flag, NaN
  propagation and default NaN, `VCVT` rounding.
- **Done when** every example and calibration ELF gives byte-identical
  reports on Unicorn and `pdsim-thumb`.

**Speed:** device-speed mode needs about 168 M device cycles per second. A
block-cached interpreter in WASM should manage roughly 100–300 M simple
instructions per second, so heavy C games may run slower than real time in
the browser. That is acceptable: timings come from the virtual clock, so the
reported numbers stay correct, and the UI should say "host slower than
device" instead of silently re-anchoring. Optimise later if needed (fusing
common pairs, avoiding `HashMap` on the hot path, and possibly a WASM-to-WASM
block JIT through `WebAssembly.Module`, though that is out of scope here).

## Phase 5: C games in the browser
1. Turn on `c-games` in `pdsim-wasm` with `pdsim-arm/thumb` and without
   `pdsim-arm/unicorn`.
2. **API layout.** `layout.rs` parses `pd_api.h` and `pd_api/*.h` at load.
   In the browser these come from the user's SDK folder (Phase 2, step 5), so
   C games need the SDK loaded once. Show a clear prompt if it's missing.
   Optional later: commit a generated layout table (member order and
   parameter kinds only, keyed by SDK version) so C games work without the
   SDK. Check that this is acceptable under the SDK license first.
3. Loading: the dropped folder must contain `pdex.elf`, or the user drops
   the `.elf` and its asset folder. `pdsim-runner::detect` already handles
   both layouts.
4. **Done when** `examples/c-stress` plays in the browser and matches the
   native report.

## Phase 6: polish
- Persist the last game and SDK in IndexedDB, so a reload goes straight back
  in.
- Gamepad API and touch controls for the d-pad, A/B and crank (drag a dial);
  `DeviceOrientation` for the accelerometer on phones.
- Download the report and trace from the page.
- A size budget: report the `.wasm` size in CI (Lua alone should be well
  under 2 MB gzipped; Capstone adds a few MB, which is another reason to
  drop it once the interpreter can classify instructions).
- Docs: a "Run in the browser" section in the README and
  [architecture.md](architecture.md).

## Out of scope
- Compiled `.pdx` bundles (Lua bytecode `.pdz` and the compiled asset formats)
  stay unsupported, as natively.
- Threaded WASM (would need COOP/COEP headers, so no GitHub Pages).
- Audio beyond the current stubs.

## Order and size (rough)

| Phase | Depends on | Size |
|---|---|---|
| 0 Spike | none | 1–2 days |
| 1 Steppable session | none | 2–3 days |
| 2 Lua in the browser | 0, 1 | 1 week |
| 3 CPU trait | none (can run in parallel with 0–2) | 2–3 days |
| 4 Thumb-2 interpreter | 3 | 3–5 weeks, mostly decoder coverage and differential testing |
| 5 C in the browser | 2, 4 | 2–3 days |
| 6 Polish | 2 (5 for C items) | ongoing |

Phases 0–2 deliver browser support for Lua games on their own and are worth
shipping before Phase 4 starts.
