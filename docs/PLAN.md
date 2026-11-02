# Plan: Playdate Performance Simulator (open source)

## Context
Panic's Playdate Simulator runs games at full desktop speed, so performance problems only show up once a game is on the device (168 MHz Cortex-M7, 16 MB RAM, 400×240 1-bit LCD). The goal is a simulator that runs the game on the dev machine **at device speed**, using a cost model calibrated on a real device, and shows live performance data while the game runs: frame time against the budget, CPU breakdown, hotspots and memory.

Decisions made with the user:
- Support **Lua and C games from the start**.
- **Rust core with a Tauri UI** (the environment already installs webkit2gtk).
- The user **has a device**, so we ship benchmark .pdx projects that produce a calibration file.

The repo (`jamesjnadeau/playdate-performance-simulator`, branch `claude/bold-volta-wapzj6`) is empty, so this is a greenfield project. License: MIT OR Apache-2.0. Never commit SDK files.

## Core idea: a virtual device clock
Nothing is timed with the wall clock. All game work is charged to a **virtual cycle ledger**:
- **Lua**: a vendored, patched Lua 5.4 (`lvm.c`) adds `cost[opcode]` to a counter on each instruction. GC work is charged through the allocator hook (bytes allocated or freed) and the GC step counts.
- **C**: the game's real **device build (`pdex.elf`, ARM Thumb-2)** runs in a Unicorn Cortex-M emulator. Each basic block is costed once by decoding it with Capstone and applying the calibrated cost per instruction class, then cached. A block hook adds the cached cost each time the block runs. This is much more accurate than scaling host time.
- **API calls** (graphics, sprites, file, sound) are native Rust. Each call is charged a calibrated `base + per_pixel·px + per_row·rows`-style cost.
- **Display**: charges the transfer cost per changed row, and sets the frame budget from `1/refreshRate` (30 fps by default, 50 max).

The frame scheduler advances virtual time. In **Device-speed mode**, the UI paces real time to virtual time, so the game looks and feels as it would on hardware: slow frames, dropped frames and all. In **Turbo mode**, the game runs as fast as possible while still reporting device timings.

## Workspace layout (Cargo workspace + Tauri app)
- `crates/pdsim-core`: virtual clock, cost ledger with categories (lua, c, gfx, gc, display, audio, io), frame scheduler, calibration loader (`calibration/*.json`: device profile Rev A/Rev B, opcode table, ARM instruction-class table, API cost table), event stream for the UI, and Chrome-trace/Perfetto JSON export.
- `crates/pdsim-gfx`: 1-bit framebuffer, drawing modes, patterns/dither, lines/rects/ellipses/polygons, images and image tables (loaded from source PNGs), `.fnt` fonts, a sprite system with collisions and dirty rects, and a pixel/row counter that feeds the cost model.
- `crates/pdsim-lua`: patched Lua 5.4 under `vendor/lua-5.4.x`, built with `cc` and linked into `mlua` (`lua54`, non-vendored, through the `LUA_LIB` env vars in the build script). Also:
  - A Playdate-dialect preprocessor: `import`, and the compound operators `+= -= *= /= //= %= <<= >>= &= |= ^=`, with line numbers kept.
  - Bindings for the `playdate.*` API: graphics, sprite, timer, frameTimer, buttons, crank, accelerometer, datastore, file, math, geometry, and stub sounds.
  - A per-function profiler that attributes the opcode cost to `source:line` / function from call-stack sampling at the cost-ledger level.
- `crates/pdsim-arm`: ELF loader and relocator for `pdex.elf`, plus the Unicorn (Thumb + MCLASS, FPv5) setup with stack and heap regions in a 16 MB RAM map. The `PlaydateAPI` struct is built in emulated memory, and each function pointer is a trampoline address that a code hook catches before dispatching to Rust. The struct layout comes from bindgen over the user's `$PLAYDATE_SDK_PATH/C_API/pd_api.h` at build time. Hotspots are attributed to functions via ELF symbols. Callbacks such as the update callback and sprite draw functions call back into the emulator.
- `crates/pdsim-cli`: a headless runner, for example `pdsim run <game-dir|pdex.elf> --frames 600 --input script.json --report out.json --trace out.trace.json`, for CI performance-regression checks.
- `app/`: Tauri v2 (`src-tauri` uses the crates; frontend in TS + Vite):
  - A screen canvas with 2×/3× scaling.
  - Keyboard input (d-pad, A/B, menu), a crank dial driven by the mouse wheel or by dragging, and accelerometer sliders.
  - A **live perf panel**:
    - A rolling frame-time bar chart stacked by category, with a budget line at 33.3 or 20 ms.
    - A CPU % gauge and a dropped-frame counter.
    - A hotspot table (Lua functions and C symbols) and a per-frame flame view.
    - A memory graph (Lua heap, images, C heap, out of 16 MB).
    - A screen overlay that flashes the rows redrawn this frame.
    - **Break on over-budget frame**, frame-by-frame stepping, and trace export.
  - Rust streams the frame stats to the UI over Tauri events at about 10 Hz, with the framebuffer sent as a 12 KB bitmask each frame.
- `calibration/bench-lua/` and `calibration/bench-c/`: Playdate projects the user builds and runs on the device. They time microbenchmarks with `playdate.resetElapsedTime/getElapsedTime` and `pd->system->getElapsedTime`, covering each Lua opcode pattern, ARM instruction classes (ALU, mul/div, load/store, FP, branch), each graphics call at several sizes, GC churn, and display flush. The results are written as JSON to the datastore and copied off in Data Disk mode. `tools/fit-calibration` (Rust bin) fits the per-unit costs with least squares and writes `calibration/rev-a.json` (and `rev-b.json`). Estimated defaults ship until the device data exists.
- `examples/`: small Lua and C sample games with known hot paths, used for validation.
- `docs/`: architecture, calibration how-to, accuracy notes.

## Milestones (each one is committed and pushed to `claude/bold-volta-wapzj6`)
1. Scaffold the workspace, CI (fmt, clippy, test), README, and license. Build `pdsim-core` (ledger, scheduler, calibration schema with default estimates).
2. `pdsim-gfx` core drawing, with golden-image tests.
3. `pdsim-lua`: patched VM with opcode costing, preprocessor, and the core `playdate.*` API. A Lua sample game runs headless through `pdsim-cli` and produces a report.
4. Tauri app: screen, input, device-speed pacing, live perf panel.
5. `pdsim-arm`: load and run the C sample `pdex.elf` with API trampolines and block costing. The same perf panel works for C games.
6. Calibration bench projects and the fitting tool. The user runs them on the device, and we load the resulting profile.
7. Fuller API coverage (sprites in depth, fonts, image tables, sound stubs with cost), plus the hotspot flame view and trace export.

## Environment fix (setup script failed, exit 4)
The SDK download line uses a Python-style placeholder that bash never substitutes, which produces `https://panic.com{playdate_version}.tar.gz`. In the environment's settings, change it to something like `wget https://download.panic.com/playdate_sdk/Linux/PlaydateSDK-${PLAYDATE_VERSION}.tar.gz`, with `PLAYDATE_VERSION=3.1.2` set above it (or use `PlaydateSDK-latest.tar.gz`). The SDK is only needed from milestone 5 on (`pd_api.h` and the `arm-none-eabi` toolchain to build `pdex.elf`). Milestones 1–4 do not need it. This session has no tool for editing the environment, so the user makes this change.

## Verification
- `cargo test --workspace`:
  - Unit tests for the ledger and scheduler math, the preprocessor (compound ops, `import`), and opcode counting (a fixed loop produces the expected opcode histogram).
  - Golden-image tests for graphics primitives.
  - ARM block costing on hand-assembled Thumb snippets.
- Headless end to end: `cargo run -p pdsim-cli -- run examples/lua-stress --frames 300 --report r.json`. Check that the frame times change as expected when the workload in the sample is doubled, and that over-budget frames are flagged.
- App: `cargo tauri dev`, play the sample, and confirm in device-speed mode that a deliberately heavy frame visibly stutters and appears in the graph and hotspot table. Screenshot it with the pre-installed Playwright/Chromium against the Vite dev frontend.
- Accuracy: once calibrated, run the example games on the device (comparing `playdate.getStats()` and frame times) and in the simulator, and record the error per category in `docs/accuracy.md`. The target is within about 15% for average frame time.
