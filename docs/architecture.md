# Architecture

## The virtual device clock

Nothing is timed with the host's wall clock. Every operation charges cycles
to a `Ledger` (`pdsim-core`) under a category: `lua`, `c`, `gfx`, `gc`,
`display`, `audio`, `io`, `other`. At the end of a frame the `Scheduler`
turns the frame's cycles into device time:

- with a refresh rate *r* (30 by default, up to 50) the budget is
  `cpu_hz / r` cycles; a frame lasts a whole number of refresh periods, so
  work of 1.01 budgets costs two periods and drops one refresh;
- with refresh rate 0 the frame lasts exactly as long as its work.

`playdate.getCurrentTimeMilliseconds`, `getElapsedTime` and the C
equivalents read this clock (including work done so far in the frame), so
games that measure themselves see device time.

The costs come from a `Calibration` profile (built in: `calibration/rev-a.json`,
fitted from device benchmarks; `calibration/default.json` holds the
hand-written estimates the fit starts from): per-opcode Lua costs, per-instruction-class ARM costs, a
`base + per_pixel + per_row + per_byte + per_item` model per API function,
and the display transfer cost.

## Lua games (`pdsim-lua`)

Lua 5.4.9 is vendored and patched (`vendor/pdsim-lua-5.4.9.patch`):

- `vmfetch` increments `pdsim_opcount[opcode]` for every instruction;
- the allocator and the incremental collector count bytes and work;
- `precallC` counts C functions called by game code.

`Ctx::sync` reads the counter deltas, multiplies them by the profile and
charges the ledger. It runs at every API call, every time query, every
`N` instructions (a count hook, for hotspot sampling) and at frame end.

The public `playdate.*` API is built in two layers:

- **Rust natives** (`src/api/{gfx,sprite,sys}.rs`, the `__pd` table) do the
  work on the simulated device and charge calibrated costs with the measured
  `Work` (pixels, rows, glyphs, sprites, bytes).
- **API-in-Lua** (`lua/api.lua`, `lua/geometry.lua`, `lua/pathfinder.lua`)
  builds the Playdate-shaped API on top: overloads, object tables with class
  metatables (CoreLibs needs `getmetatable(img) == gfx.image` and
  `setmetatable` on sprites), input handlers, files, JSON. These chunks are
  named `=[pdsim]...`; the patched VM counts their instructions in a separate
  histogram and does not count the C functions they call, so only the native
  cost they charge reaches the ledger.

`import` resolves game files, then `CoreLibs/...` from the SDK (or small
stand-ins in `lua/corelibs/` when there is no SDK). The source is run
through the dialect preprocessor (compound assignment operators), keeping
line numbers.

Hotspots: at each sample the Lua cycles since the last sample are attributed
to the current call stack (`name source:line` frames). Native API costs are
attributed to the calling Lua function with a `[graphics.fillRect]` leaf.

## C games (`pdsim-arm`)

- `elf.rs` loads `pdex.elf` at `0x60000000` (as the device does) and applies
  its `R_ARM_ABS32` relocations (it is linked at 0 with `--emit-relocs`).
- `layout.rs` reads the `PlaydateAPI` struct tree from the SDK headers at load
  time: member order (the layout) and each function's parameter kinds.
- `emu.rs` maps code, heap (8 MB), stack, the API structs and a page of
  trampolines into a Unicorn Cortex-M7. Every API function pointer points at
  a trampoline; a code hook decodes the call (`abi.rs`, AAPCS-VFP: ints in
  r0–r3 and the stack, floats in s0–s15, `PDRect` as a float aggregate,
  varargs as doubles), runs the Rust implementation (`api.rs`) and returns
  by setting `PC = LR`.
- A block hook costs each basic block once (`cost.rs`: Capstone decode,
  calibrated cost per instruction class, extra cost per register for
  push/pop/ldm/stm) and adds the cached cost each time it runs. Blocks that
  end in a call or a return drive a shadow call stack, so cycles are
  attributed to ELF symbols.
- Callbacks (the update callback, sprite update/draw/collision functions,
  `listfiles`) call back into emulated code with a nested `emu_start`.
- `getFrame` mirrors the framebuffer into emulated memory and syncs it around
  graphics calls and at frame end.

## Shared device state (`pdsim-device`, `pdsim-gfx`)

`Device` holds the ledger, scheduler, graphics, sprites, input, the file
sandbox (reads: data folder, then game folder; writes: data folder) and the
optional feature recorder used by calibration. `pdsim-gfx` draws into a
400×240 1-bit framebuffer (52-byte rows like the device), tracks dirty rows
for the display cost, and implements the sprite system's dirty-rect redraw
plan and collision resolution as pure data so that backends can run game
callbacks without holding borrows.

## Front ends

- `pdsim-session` runs a game on its own thread. In device-speed mode it
  sleeps so that wall time follows device time; it emits JSON events
  (`frame` with the 12 KB framebuffer, `stats` ~10×/s, `flame`, `log`, …)
  and accepts commands (`load`, `play`, `pause`, `step`, `button`, `crank`,
  `breakOnOverBudget`, `exportTrace`, …).
- The Tauri app (`app/`) relays these over IPC; `pdsim serve` relays them over
  WebSocket so the same frontend runs in a browser.
- `SessionCore` (in `pdsim-session`) is the session without threads, channels
  or `Instant`: the host calls `handle(command, now_ms)` and `tick(now_ms)` and
  collects the events. The threaded `Session` is a thin native wrapper around
  it. `pdsim-wasm` (bin `pdsim-web`, built for `wasm32-unknown-emscripten`)
  exposes the core through a four-function C ABI; the app runs it in a Web
  Worker as a third transport, `kind: "wasm"`, speaking the same
  command/event protocol, so the UI is unchanged. Emscripten supplies the
  in-memory filesystem, so games and the SDK are copied in before `load`, and
  traces come back as `traceData` events (the page downloads them) instead of
  files. The build is single-threaded; Lua only. See `docs/wasm-port.md`.
- `pdsim run` drives a game headless and writes reports and traces.

## In the browser

The web build is the same app (`app/`) talking to `pdsim-web` in a Web Worker
(`app/src/wasm/worker.ts`) instead of to a native session:

- **Pacing.** The worker calls `pdsim_tick(now)` in slices of up to 8 ms,
  then sleeps with `setTimeout` until the next frame is due, or yields through
  a `MessageChannel` when work is already due, so commands are never starved.
  The session reports `realtime` (device ms per wall ms) in its stats; the
  page shows it as "Speed (real time)" and warns when device mode falls
  behind.
- **Files.** Games and the SDK are copied into Emscripten's in-memory
  filesystem under `/game` and `/sdk`; save data goes to `/data/<game>`.
  All three are mounted on IDBFS and synced to IndexedDB after a load, on
  pause or step, and once a second while running. On start the worker syncs
  them back in: an SDK loaded once stays loaded, and if `/game` holds files it
  sends a `restored` event so the page reopens the last game. A game that
  killed the worker is remembered in `localStorage` and not reopened.
- **Input.** Keyboard, the on-page d-pad, buttons and crank dial (pointer
  events, so touch works), gamepads through the Gamepad API (polled each
  animation frame; the right stick sets the crank angle), and, on phones,
  `devicemotion` for the accelerometer once "Use device tilt" is ticked. The
  pure mappings are in `app/src/input.ts`.
- **Size.** CI fails if `pdsim_web.wasm` grows past 2 MiB gzipped
  (`scripts/wasm-size.sh`).
