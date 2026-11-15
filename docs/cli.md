# `pdsim` command reference

```bash
cargo build --release -p pdsim-cli     # binary: target/release/pdsim
```

## `pdsim run`

Runs a game headless for a fixed number of frames and reports device
timings.

```text
pdsim run <GAME> [options]
```

`GAME` can be any of:

- a folder holding `main.lua` (a Lua game's `Source/` folder);
- a folder whose `Source/` holds `main.lua` or `pdex.elf`;
- a folder holding `pdex.elf`, or the `pdex.elf` itself (a C game);
- a `main.lua` file.

Games run from source: PNG images and `.fnt` fonts, not a compiled `.pdx`.

| Option | Default | What it does |
|---|---|---|
| `--frames N` | 300 | Frames to simulate. At 30 fps, 300 frames is 10 s of device time. |
| `--input FILE` | none | Scripted input, see [Input scripts](#input-scripts). |
| `--report FILE` | none | Write the [JSON report](#report-json). |
| `--trace FILE` | none | Write a Chrome/Perfetto trace in device time (open in `ui.perfetto.dev` or `chrome://tracing`). |
| `--frames-out FILE` | none | Write [per-frame stats](#per-frame-stats-jsonl) as JSON lines. |
| `--screenshot FILE` | none | Save the last frame as a 400×240 PNG. |
| `--calibration FILE` | built-in (`calibration/rev-a.json`) | Calibration profile, e.g. your own `rev-b.json`, or `calibration/default.json` for the unfitted estimates. |
| `--data-dir DIR` | `$TMPDIR/pdsim-data/<game>` | Where the game's saved data lives (`playdate.datastore`, `playdate.file` writes). Reads look here first, then in the game folder. |
| `--sdk DIR` | `$PLAYDATE_SDK_PATH` | Playdate SDK, for CoreLibs, the system font and (C games) `pd_api.h`. |
| `--cpu unicorn\|thumb` | `unicorn` if built in, else `thumb` | CPU emulator for C games. `thumb` is the pure-Rust interpreter and needs a build with `--features thumb`, and `unicorn` needs `--features unicorn` (an alias for the default `c-games`); both give byte-identical reports. Asking for a backend this build lacks prints `pdsim: this build has no <name> CPU (rebuild with --features <name>)` and exits 2. |
| `--no-profile` | off | Skip per-function attribution. Faster; the report has no hotspots. |
| `--max-over-budget N` | none | Exit 1 if more than `N` frames go over budget. |
| `--max-avg-ms X` | none | Exit 1 if the average frame work exceeds `X` ms. |
| `-q`, `--quiet` | off | Don't print the game's console output (`print`, `pd->system->logToConsole`). The summary is still printed. |

### Output streams and exit status

- **stdout**: the game's console output (unless `-q`).
- **stderr**: the game description line, then the summary.
- **exit 0**: the run finished and every limit held.
- **exit 1**: the game failed to load, raised an error (the message and Lua
  traceback are printed, and the report/trace still cover the frames that
  ran), or a `--max-*` limit was exceeded.

Summary example:

```text
300 frames, 32300.0 ms device time, budget 33.33 ms
work avg 85.30 ms  p50 44.06  p95 44.06  max 2566.40 (frame 179)
over budget: 300 frames, dropped refreshes: 669, effective fps 9.3
avg per category (ms): lua 71.69, gfx 5.43, gc 7.55, display 0.62
peak memory: 3890.2 KB
hotspots (self cycles):
    1841520977  physics main.lua:46
```

- **work** is the time the CPU was busy in a frame. **budget** is one
  display refresh (`1000 / refreshRate` ms; 33.33 ms at the default 30 fps,
  0 when the game sets refresh rate 0).
- A frame whose work exceeds the budget is **over budget** and holds the
  screen for an extra refresh period; each missed refresh is a **dropped
  refresh**.
- `(ESTIMATED calibration)` appears when the profile is not device-fitted
  (such as `calibration/default.json`). Then compare relative numbers until
  you [calibrate](calibration.md).

### Input scripts

```json
{
  "events": [
    { "frame": 10, "press": ["a", "right"] },
    { "frame": 40, "release": ["a"] },
    { "frame": 60, "crank": 15.0 },
    { "frame": 90, "crank_to": 90.0, "dock": false },
    { "frame": 120, "accel": [0.0, 1.0, 0.0] }
  ],
  "crank_per_frame": 0.0
}
```

| Field | Meaning |
|---|---|
| `frame` | Frame index (from 0) the event applies at, before the game's update runs. |
| `press` / `release` | Button names: `a`, `b`, `up`, `down`, `left`, `right` (case-insensitive; unknown names are ignored). A pressed button stays held until released. |
| `crank` | Turn the crank by this many degrees (negative = backwards). |
| `crank_to` | Set the crank to this absolute angle. |
| `dock` | `true` docks the crank, `false` undocks it. |
| `accel` | Accelerometer reading `[x, y, z]` in g. |
| `crank_per_frame` | Degrees to turn the crank on every frame. |

There is no menu button: the system menu is not simulated.

### Report JSON

`--report` writes one object:

| Field | Type | Meaning |
|---|---|---|
| `calibration` | string | Profile name. |
| `estimated_calibration` | bool | `true` when the profile is not device-fitted. |
| `frames` | int | Frames simulated (fewer than `--frames` if the game errored). |
| `device_time_ms` | float | Device time covered by the run. |
| `budget_ms` | float | Frame budget at the end of the run. |
| `avg_work_ms`, `p50_work_ms`, `p95_work_ms`, `max_work_ms` | float | CPU busy time per frame. |
| `max_work_frame` | int | Index of the worst frame. |
| `over_budget_frames` | int | Frames whose work exceeded the budget. |
| `over_budget_frame_indices` | int[] | Their indices (first 1000). |
| `dropped_frames` | int | Display refreshes missed. |
| `effective_fps` | float | Frames shown per second of device time. |
| `avg_category_ms` | object | Average busy ms per category: `lua`, `c`, `gfx`, `gc`, `display`, `audio`, `io`, `other`. |
| `peak_memory` | int | Peak bytes in use (Lua heap + images + C heap). |
| `hotspots` | array | Top 25 functions: `{name, self_cycles, total_cycles}`. |

Hotspot names are `function source:line` for Lua (`physics main.lua:46`),
the ELF symbol for C (`update_particles`), and `[namespace.call]` for time
spent inside an API call (`[graphics.fillRect]`, `[sprite.update]`). `[gc]`
is garbage collection; `(toplevel)` is code outside any function.
Cycles convert to ms at the profile's CPU clock (168 MHz by default:
168,000 cycles = 1 ms).

### Per-frame stats (JSONL)

`--frames-out` writes one object per line:

```json
{"index":0,"start_ms":0.0,"work_ms":9.27,"frame_ms":33.33,"budget_ms":33.33,
 "cpu_percent":27.8,"categories":{"lua":8.07,"gfx":0.57,"gc":0.005,"display":0.62,"c":0.0,"audio":0.0,"io":0.0,"other":0.0},
 "cycles":[1355547,0,96215,922,104800,0,0,0],"over_budget":false,"dropped":0,
 "rows_updated":240,"memory":{"lua_heap":621795,"images":40112,"c_heap":0,"capacity":16777216},
 "profile":{"folded":{"update main.lua:75;update main.lua:39":27535}}}
```

`cycles` is in category order (`lua, c, gfx, gc, display, audio, io,
other`). `rows_updated` is how many display rows changed (0–240).
`profile.folded` maps `;`-separated call stacks to self cycles, the format
flame-graph tools take; it is absent with `--no-profile`.

## `pdsim serve [GAME] [--port 8765]`

Serves a live session over WebSocket on `127.0.0.1` for the web build of
the app (`npm run dev` in `app/`). If `GAME` is given, it loads when a UI
connects. A plain browser runs the simulator in-page by default, so open the
app with `?backend=ws` (or `?ws=ws://host:port`) to use this server. See
[session-protocol.md](session-protocol.md) for the messages.

## `pdsim default-calibration`

Prints the built-in profile (`calibration/rev-a.json`) as JSON: a starting
point for hand-edited profiles. `fit-calibration` starts from the estimates in
`calibration/default.json` instead.
