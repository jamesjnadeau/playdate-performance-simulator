# Testing your game's performance

This guide is for running *your own* Playdate game in the simulator to find
slow frames, see what causes them, and keep them from coming back. Flags and
file formats are in [cli.md](cli.md).

## 1. Set up

```bash
export PLAYDATE_SDK_PATH=~/PlaydateSDK          # wherever your SDK is
cargo build --release -p pdsim-cli
alias pdsim=$PWD/target/release/pdsim
```

The SDK is optional for Lua games: without it, `import "CoreLibs/..."`
loads small stand-ins (`crates/pdsim-lua/lua/corelibs/`) whose costs are
less accurate and which lack some functions, and text uses a fallback font.
Set it whenever you have it. C games need it (the simulator reads
`C_API/pd_api.h` to lay out the API).

## 2. Point it at the game

**Lua**: pass the project folder or its `Source/` folder, the one holding
`main.lua`. The simulator reads source files, PNG images, image tables
(`name-table-W-H.png` grids or `name-1.png`, `name-2.png`, …) and `.fnt`
fonts. It does not read `.pdx` bundles or `.pdz`/`.pdi`/`.pft` compiled
assets.

**C**: build the device binary (`make device_bin` with the SDK's Makefile
and `arm-none-eabi-gcc`), then pass the folder that holds `pdex.elf`, or
the file itself. Assets are read from the same folder as `pdex.elf`, so
copy or link them next to it if they live in `Source/`. The game must
register an update callback with `pd->system->setUpdateCallback`; Lua+C
hybrid games are not supported.

```bash
pdsim run path/to/MyGame --frames 300
```

If it fails to load, the error names the missing file or API. Check
[accuracy.md](accuracy.md#what-is-not-yet) for features that are not
simulated (sound is silent, network and scoreboards always fail, no system
menu, no video).

## 3. Drive it with an input script

Without input, most games sit on a title screen, which tells you little.
Write a script that reaches the part you care about and plays it:

```json
{
  "events": [
    { "frame": 30,  "press": ["a"] },
    { "frame": 32,  "release": ["a"] },
    { "frame": 90,  "press": ["right"] },
    { "frame": 400, "release": ["right"] },
    { "frame": 200, "press": ["a"] },
    { "frame": 202, "release": ["a"] }
  ],
  "crank_per_frame": 0
}
```

```bash
pdsim run MyGame --frames 600 --input level1.json \
  --report level1.report.json --frames-out level1.frames.jsonl \
  --screenshot level1.png
```

Check `level1.png` to confirm the script reached the scene you meant to
test. Timing is deterministic: the same game, script and profile give the
same frames every run, so scripts make reliable regression tests. Saved
data persists between runs in the data folder; pass a fresh
`--data-dir` per run when the game should start from a clean save.

To capture several moments, run the same script with increasing
`--frames` and a screenshot each time.

## 4. Read the results

Start with the summary line `over budget: N frames, dropped refreshes: M`.
Over-budget frames are what players see as stutter.

1. **Which frames?** `over_budget_frame_indices` in the report, or filter
   the JSONL:

   ```bash
   jq -c 'select(.over_budget) | {index, work_ms, categories}' level1.frames.jsonl
   ```

   A single spike every N frames is usually a periodic job or a GC
   burst; a long run of slow frames is a sustained workload (a busy scene,
   too many sprites).

2. **What category?** `avg_category_ms` and each frame's `categories`:
   - `lua`: bytecode execution, i.e. your game logic.
   - `gfx`: drawing and sprite system work (pixels touched, sprites
     updated, collisions).
   - `gc`: allocation and garbage collection. High `gc` means the frame
     allocates many short-lived tables, strings or closures.
   - `display`: transferring changed rows to the screen; at most ~0.6 ms
     with the estimates, so it only matters at 50 fps.
   - `c`: native code in C games.
   - `io`: file and datastore access.

3. **What code?** `hotspots`, ranked by self cycles. Lua entries name the
   function and the line it starts on; bracketed entries are API calls,
   attributed to the Lua function that made them (see the folded stacks in
   `profile.folded` per frame, or open `--trace` in
   [Perfetto](https://ui.perfetto.dev)). Divide cycles by 168,000 for ms.

4. **Memory**: `peak_memory` against the 16 MB device (minus what the
   OS uses). A Lua heap that keeps climbing across frames is a leak.

## 5. Make a fix and compare

Change one thing, rerun the same script and compare:

```bash
jq '{avg_work_ms, p95_work_ms, max_work_ms, over_budget_frames, avg_category_ms}' before.json after.json
```

With the estimated profile, trust **relative** changes (this version is
30% faster, this frame is the worst, this function dominates) more than
absolute milliseconds, which can be off by several times on Lua-heavy
frames. For absolute numbers, pass a fitted profile (`--calibration
calibration/rev-a.json` for a Rev A device) or [calibrate](calibration.md)
against your own.

## 6. Guard it in CI

```bash
pdsim run MyGame --frames 900 --input level1.json -q \
  --max-over-budget 0 --max-avg-ms 20 --report perf.json
```

The command exits 1 when a limit is exceeded or the game raises an error,
so it can gate a pull request. Keep `perf.json` as a build artifact to see
what changed. Give the limits some headroom when using the estimated
profile.

## Watching it live

The desktop app shows the same data while you play:

```bash
cd app && npm install
npx tauri dev -- -- ../path/to/MyGame
```

Keys: arrows = d-pad, `X`/`K` = A, `Z`/`J` = B, space = play/pause,
`.` = step one frame. The mouse wheel or dragging the dial turns the crank.
Turn on **break on over-budget frame** to stop at the first slow frame and
see its flame view.

Without Tauri, run `pdsim serve MyGame` and `npm run dev` in `app/`, then
open the Vite URL in a browser.
