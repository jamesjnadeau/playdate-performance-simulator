# GUI runs C games in slow motion: optimised dev builds and a real-time indicator

## Why

Running CrankSail (`playdate-sailing-sim`) through its `tools/sim_performance.sh`
(`npx tauri dev`) showed the game screen updating very slowly, although the
game fits its budget (24.8 ms of 33 ms, a steady 30 fps on the device).
The display path is not the cause. The simulator cannot produce frames as fast
as real time:

| Build | Host time for 600 frames (20 s of device time) |
|---|---|
| release | 20.2 s (about 1×) |
| debug (what `tauri dev` builds) | 180 s (about 0.11×) |

`tauri dev` compiles everything at `opt-level = 0`, including Unicorn (C, built
through `cmake`, which picks `Debug` from the opt level) and pdsim's per-block
cycle hook. Meanwhile the FPS tile shows *device* fps (30), so the
slowdown looks like a display problem.

## Goals

1. `tauri dev` (and `cargo run`/`cargo test` without `--release`) runs games
   about as fast as a release build, so device-speed mode is real time for a
   game like CrankSail.
2. The GUI says plainly when the simulator can't keep up with real time, so
   slow motion is never mistaken for game performance.

## Non-goals

- Making the emulator itself faster (per-block hook work). That is a later,
  separate project.
- Changing how the screen is sent or drawn.
- Changing `sim_performance.sh` in the game's repo. Fix 1 makes it fast as it is.

## Anti-goals

- Making the indicator nag in Turbo mode, or while paused or stepping.
- Making dev rebuilds of pdsim's own crates much slower to compile.

## Approaches considered

1. **Optimise dev builds in the workspace `Cargo.toml`** (chosen). This fixes
   every dev entry point at once: `tauri dev`, `cargo run`, `cargo test`.
2. Have launch scripts use `tauri dev --release` or a prebuilt binary. This only
   fixes the scripts that do it, and every downstream script (like
   CrankSail's) would have to change.
3. An emulator hot-path rewrite. Too big for now (see non-goals).

Paired with a **real-time indicator** in the perf panel (chosen). The
alternative, changing the FPS tile to host fps, would lose the device fps,
which is the number the tool exists to show.

## Design

### 1. Dev profile (workspace `Cargo.toml`)

```toml
[profile.dev.package."*"]   # all dependencies, Unicorn among them
opt-level = 3

[profile.dev]               # pdsim's own crates
opt-level = 1
```

Debug info and debug assertions stay on. Dependencies rarely rebuild, so
`opt-level = 3` there costs nothing day to day. For pdsim's own crates,
`opt-level = 1` keeps incremental compiles fast. The builder may raise it to 2
for the hot crates (`pdsim-arm`, `pdsim-lua`, `pdsim-core`, `pdsim-gfx`,
`pdsim-device`) via `[profile.dev.package.<name>]` if measurement shows that
matters. Success means the debug sailing run (command below) takes no more than
about 1.5× the release time.

### 2. Real-time indicator

**Session** (`crates/pdsim-session`): each `stats` event gains `realtime`,
which is device ms advanced ÷ wall ms elapsed since the previous `stats` while
running. Set it to `null` when paused, after a step, or when less than about
50 ms of wall time passed. It is reported in Turbo too (for example "12×"),
since that is useful there. Document it in `docs/session-protocol.md`.

**UI** (`app/`): a new tile after "FPS (device)":

```
┌ Speed ─────────┐
│ 0.11× real time│   ← warning colour (the existing .bad style) in Device mode when < 0.9×
└────────────────┘
```

It shows `–` when `realtime` is null, and is smoothed over the last few stats
events so it doesn't flicker. In Device mode at full speed it reads `1.00×`.
In Turbo it shows the multiplier with no warning colour. The tile's tooltip
explains: "How fast the simulation runs compared with the device. Below 1×
the game is in slow motion because the host can't keep up; device timings
are still correct."

## Verification

- Before/after timing of the CrankSail sailing run in a debug build:
  `PLAYDATE_SDK_PATH=… target/debug/pdsim run <sailing-sim>/Source --frames 600 --input <sailing-sim>/tools/perf_free_sail.json -q`.
- Session unit test: `stats` carries `realtime` about 1 in device mode for a
  light game, greater than 1 in turbo, and null when paused.
- GUI check: the tile reads about `1.00×` on `examples/lua-stress` in Device
  mode, and drops below 1 with a warning when the host is slowed down.

## Left to the builder

- The exact smoothing window and threshold (about 0.9 is fine).
- Tile wording and styling within the existing tile pattern.
