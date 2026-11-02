# Calibrating against a real Playdate

The simulator charges every operation a cost in device cycles. Until you
calibrate, it uses the built-in estimates in `calibration/default.json`, and
every report says **ESTIMATED**. Calibrating fits those costs to timings from
your own device.

## How it works

The two bench projects time many small benchmarks with
`resetElapsedTime()`/`getElapsedTime()` (Lua) and
`pd->system->resetElapsedTime()`/`getElapsedTime()` (C):

- `calibration/bench-lua`: Lua VM patterns (arithmetic, tables, calls,
  closures, strings, GC churn, …), graphics calls at several sizes, image
  drawing, text and sprites.
- `calibration/bench-c`: inline-assembly loops for each ARM instruction class
  (ALU, multiply, divide, load, store, load-multiple, branch, call, FP add,
  FP divide, FP square root, FP load) and C graphics calls.

`fit-calibration` then runs the *same* projects in the simulator with
feature recording on. At each `getElapsedTime` the simulator records exactly
what ran since the matching reset: the Lua opcode histogram, C library calls,
bytes allocated and freed, GC work, every API call with the pixels/rows/items
it touched, and the ARM instruction classes executed. Each device timing is
then one equation:

    device_cycles ≈ Σ feature_count × cost_per_unit

and the fitter solves for the costs by weighted least squares on relative
error, with a weak pull toward the starting profile (so costs the benchmarks
cannot tell apart keep their prior) and costs kept non-negative.

## Steps

1. Build the bench projects with the SDK:

   ```bash
   pdc calibration/bench-lua/Source bench-lua.pdx
   make -C calibration/bench-c device     # needs arm-none-eabi-gcc
   ```

2. Put both `.pdx` on the Playdate (Sideload or Data Disk mode) and run each
   until the screen says **Done** (a few minutes). Do not touch the device
   while they run.

3. Put the device in Data Disk mode and copy:
   - `Data/dev.pdsim.benchlua/results.json` → `lua-results.json`
   - `Data/dev.pdsim.benchc/results.json` → `c-results.json`

4. Fit:

   ```bash
   cargo run --release -p fit-calibration -- \
     --lua-results lua-results.json --c-results c-results.json \
     --device rev-a --out calibration/rev-a.json --report fit-report.json
   ```

   It prints each benchmark's device time and the model's prediction before
   and after fitting, and the mean error.

5. Use the profile:

   ```bash
   cargo run -p pdsim-cli -- run my-game --calibration calibration/rev-a.json
   ```

   (In the app, set the profile in the session `load` command; a picker is a
   to-do.) Please consider contributing `rev-a.json`/`rev-b.json` back.

Rev A and Rev B devices differ (CPU revision and memory), so fit each
separately and pass `--device rev-b` for a Rev B.

## Benchmark configuration

Both projects read an optional `bench-config.json` from their data folder:
`{"scale": 1, "repeats": 3}`. `scale` multiplies iteration counts (raise it
if timings are very short); `repeats` runs each benchmark several times. The
results file records both, and the fitter replays the bench with the same
settings.

## Checking the pipeline without hardware

`tools/fit-calibration/tests/pipeline.rs` runs the benches in the simulator
under a deliberately perturbed profile (playing the device), then fits from
the defaults. The fit has to bring the error from >10% to <3% and recover the
perturbed costs that the benchmarks identify. It also checks that what the
game measured with `getElapsedTime` is fully explained by the recorded
features.

## What is not calibrated yet

- Display transfer cost: benchmarks run within one frame, so the display
  terms (`display.base`, `display.per_row`) keep their estimates unless a
  timing spans frames.
- Sound: sound is stubbed; its costs are flat estimates.
- Branch-taken penalties and cache effects on the M7 are folded into the
  per-class averages.
