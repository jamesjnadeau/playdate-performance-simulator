---
name: playdate-perf-test
description: Use when asked to test, profile, benchmark or find slow frames, stutter, frame drops, lag, GC spikes or memory growth in a Playdate game (Lua main.lua project or C pdex.elf), to check whether a change made a Playdate game faster, or to add a performance gate to a Playdate game's CI, using the pdsim simulator in this repository.
---

# Testing a Playdate game's performance with pdsim

`pdsim` runs a Playdate game headless against a virtual device clock and
reports how long each frame would take on hardware. Runs are deterministic:
the same game, input script and profile give identical numbers, so a
before/after comparison needs one run of each.

Full references: `docs/testing-games.md` (walkthrough), `docs/cli.md`
(every flag, report and JSONL field), `docs/accuracy.md` (what is not
simulated).

## 1. Build and set up

```bash
cd <this repo>
cargo build --release -p pdsim-cli          # → target/release/pdsim
export PLAYDATE_SDK_PATH=~/Developer/PlaydateSDK-3.1.2   # or wherever it is; check `ls ~/Developer ~/PlaydateSDK 2>/dev/null`
```

- Lua games run without the SDK (stand-in CoreLibs, fallback font) but are
  less accurate; set it if it exists.
- C games need the SDK (`C_API/pd_api.h`) and an ARM build:
  `make device_bin` in the game folder (needs `arm-none-eabi-gcc`), which
  produces `pdex.elf`. Assets must sit in the same folder as `pdex.elf`.

## 2. Identify what to pass as GAME

| Game layout | Pass |
|---|---|
| `MyGame/Source/main.lua` | `MyGame` or `MyGame/Source` |
| `MyGame/main.lua` | `MyGame` |
| C game with `pdex.elf` | the folder holding it, or the file |
| Only a `.pdx` | not supported: find the source project |

Lua+C hybrid games are not supported. Run a short smoke test first:

```bash
target/release/pdsim run GAME --frames 30 -q
```

A load error names the missing file or API; check `docs/accuracy.md` before
assuming it is a simulator bug.

## 3. Write an input script that reaches real gameplay

Without input most games sit on a title screen, and the numbers mean
nothing. Read the game's update code to learn which buttons start it and
what moves the player, then write a script. If the game reads no input
(no `buttonIsPressed`/`buttonJustPressed`/`getCrank*` calls), skip
`--input`, but still check the screenshot.

```json
{
  "events": [
    { "frame": 30, "press": ["a"] },
    { "frame": 32, "release": ["a"] },
    { "frame": 60, "press": ["right"] },
    { "frame": 240, "release": ["right"] }
  ],
  "crank_per_frame": 0
}
```

- Buttons: `a b up down left right`. There is no menu button.
- A pressed button stays held until a `release` event.
- Other events: `crank` (relative degrees), `crank_to` (absolute),
  `dock` (bool), `accel` (`[x, y, z]` in g). `crank_per_frame` turns the
  crank every frame (crank-driven games).
- `frame` is 0-based and matches `index` in the per-frame JSONL; an event
  applies before that frame's update runs. Hold presses for at least 2
  frames.

## 4. Pick a profile

The built-in profile is a rough estimate (the summary says ESTIMATED). It
can be off by 5x or more on Lua-heavy frames, enough to flip "a few
stutters" into "never reaches 30 fps". Use a fitted profile whenever one
exists: `ls calibration/*.json` (`default.json` is the estimate;
`rev-a.json` is fitted to a Rev A device). Profiles are per hardware
revision. If you don't know the user's revision, run with each available
profile and report both, naming which is which. With no fitted profile
for their revision, say that absolute times are unverified.

## 5. Run and confirm the scene

```bash
target/release/pdsim run GAME --frames 600 --input play.json -q \
  --calibration calibration/rev-a.json \
  --report run.json --frames-out run.jsonl --screenshot run.png \
  --data-dir "$(mktemp -d -p "$SCRATCH")"
```

**Always look at the screenshot** (Read tool on `run.png`) before trusting
the numbers. If it shows a title or menu, the script did not reach
gameplay: fix it and rerun. For several checkpoints, rerun with smaller
`--frames` values and a screenshot each.

A fresh `--data-dir` starts from a clean save (`$SCRATCH` is your scratch folder). Omit it only when
you want saved progress to carry over between runs.

The summary goes to stderr; `-q` hides the game's `print` output (drop `-q`
when debugging the script). Exit 1 means the game errored (message and
traceback printed) or a `--max-*` limit was hit.

## 6. Read the results

```bash
jq '{avg_work_ms, p95_work_ms, max_work_ms, max_work_frame, budget_ms,
     over_budget_frames, dropped_frames, effective_fps, avg_category_ms,
     peak_memory, estimated_calibration}' run.json
jq '.hotspots[:10]' run.json
jq -s -c 'sort_by(-.work_ms) | .[:10][] | {index, work_ms, categories}' run.jsonl
jq -c 'select(.over_budget) | .index' run.jsonl | tr '\n' ' '
```

When every frame is over budget, `select(.over_budget)` tells you nothing;
sort by `work_ms` to find the spikes. `max_work_frame` picks one of several
tied frames, so look for the pattern rather than trusting that one index.

- **Budget**: 33.33 ms at 30 fps. A frame whose `work_ms` exceeds
  `budget_ms` is over budget and shows as stutter.
- **Pattern**: one spike every N frames is a periodic job or GC burst; a
  run of slow frames is a sustained load.
- **Categories**: `lua` game logic, `gfx` drawing and sprites, `gc`
  allocation/collection churn, `display` screen transfer (≤ ~0.6 ms), `c`
  native code, `io` files/datastore.
- **Hotspots** rank by `self_cycles` (168,000 cycles = 1 ms):
  `physics main.lua:46` is a Lua function and the line it starts on,
  `[graphics.fillRect]` is time inside an API call, `[gc]` is the
  collector, `(toplevel)` is code outside functions; C entries are ELF
  symbols. To see which function made an API call, inspect a slow frame's
  stacks:
  `jq -c 'select(.index==N) | .profile.folded | to_entries | sort_by(-.value) | .[:10]' run.jsonl`.
  Cycles convert at the profile's `cpu_hz`; a different profile changes
  the cycle counts, not the rate.
- **Memory**: `peak_memory` vs the 16 MB device. The Lua heap climbs
  between collections, then drops (a sawtooth); that is normal. A leak is
  when the low points after each drop keep rising:
  `jq -c '[.index, .memory.lua_heap]' run.jsonl`.

## 7. Report honestly

- Name the profile behind every number. With `estimated_calibration:
  true`, say the milliseconds may be far off and lead with relative
  findings (which frames, which function, how much faster). A fitted
  profile is device-grade only for its hardware revision.
- Sound, network, scoreboards, the system menu and video are not
  simulated; do not draw conclusions about them.
- Quote the exact command, script and frame count so the user can rerun it.

## 8. Compare a fix

Rerun the *identical* command (same script, frames, profile) and diff:

```bash
jq -s 'map({avg_work_ms, p95_work_ms, max_work_ms, over_budget_frames, avg_category_ms})' before.json after.json
```

One run per side is enough (deterministic). Change one thing at a time.

## 9. Gate CI

```bash
target/release/pdsim run GAME --frames 900 --input play.json -q \
  --calibration calibration/rev-a.json --max-over-budget 0 --max-avg-ms 20 --report perf.json
```

Exits 1 when a limit is exceeded. Set limits from a measured baseline with
headroom, keep `perf.json` as an artifact, and commit the input script.

## Common mistakes

| Mistake | Fix |
|---|---|
| Profiling the title screen | Write an input script; check the screenshot |
| Passing a `.pdx` | Pass the source folder |
| C assets in `Source/` but `pdex.elf` elsewhere | Copy/link assets next to `pdex.elf` |
| Reporting ESTIMATED ms as device truth | Use a fitted profile; otherwise say the ms may be far off |
| Listing `select(.over_budget)` when all frames are over | Sort by `work_ms` to find the spikes |
| Calling a heap sawtooth a leak | Compare the heap minima after each GC drop |
| Comparing runs with different scripts or `--frames` | Keep everything identical except the change |
| Stale save data changes behavior | Use a fresh `--data-dir` |
| Reading hotspots by `total_cycles` first | Rank by `self_cycles`; use totals for call trees |
| Running many times to "average out noise" | Not needed: runs are deterministic |
