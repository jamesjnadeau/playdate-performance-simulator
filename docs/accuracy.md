# Accuracy notes

## Status

The built-in profile is `calibration/rev-a.json`, fitted from 213 benchmark
runs on a Rev A Playdate (71 benchmarks; `calibration/device-results/`). The
fit brings the mean benchmark error from 70% (the hand-written estimates in
`calibration/default.json`) to 3.5%. The worst-fitting benchmarks are listed
in `calibration/device-results/fit-report.json`: `table-array-get` (23%), the
short ARM ALU, loop and branch loops (10–21%), `c-clear` (11%) and
`sprite-update-50` (10%).

That is per benchmark. Whole games have not yet been timed on the device;
the target is average frame time within about 15% of the device per game.
Please add rows below when you measure one.

| Game | Device | Profile | Avg frame (device) | Avg frame (sim) | Error |
|---|---|---|---|---|---|
| examples/lua-stress | – | rev-a | – | 85.3 ms | – |
| examples/c-stress | – | rev-a | – | 5.2 ms | – |

With the old estimates the same games ran at 11.6 ms and 4.8 ms: the
estimates undercharged Lua by several times.

The display transfer and sound costs in `rev-a.json` are still the estimates
(see [calibration.md](calibration.md#what-is-not-calibrated-yet)). There is no
Rev B profile; on a Rev B, treat absolute times as approximate.

To compare: run the game on the device with `playdate.getStats()` /
`playdate.getElapsedTime()` around `playdate.update`, and run the same frames
in `pdsim run --frames-out`.

## What is modeled

- **Lua**: every opcode, at a per-opcode cost; C library calls at a
  per-function cost (`string.format` costs more than `math.abs`); allocation/free volume and incremental GC work. The VM and GC are the
  real Lua 5.4 code, so allocation patterns and collection cycles match the
  device's Lua (Playdate uses Lua 5.4 as well).
- **C**: every executed instruction of the real device build, by class; the
  same compiler output that runs on the device.
- **API calls**: a per-function model over the work actually done (pixels,
  rows, glyphs, sprites, bytes), measured by the simulator's own
  implementation.
- **Display**: per update plus per changed row; refresh-rate budgeting and
  dropped refreshes.
- **Memory**: Lua heap, images and fonts, and the C heap, against 16 MB.

## What is not (yet)

- Caches, flash wait states and branch prediction are folded into averages,
  so tight loops whose working set fits the M7's caches may differ most.
- Sound is stubbed (objects exist and `play` charges a flat cost); no audio
  is produced and the audio thread's CPU use is not modeled.
- C library functions the benchmarks don't call fall back to one average
  cost, and a call's cost doesn't depend on its arguments (a long
  `string.format` costs the same as a short one).
- Graphics run the simulator's own rasterizers. Pixel output matches the
  device for the common cases (see the golden tests), but some details
  (ellipse edge pixels, text kerning edge cases, `drawSampled`, blur) are
  approximations, and rotated/scaled drawing uses nearest-neighbour sampling.
- Not simulated: video, networking, scoreboards, the system menu, Lua+C
  hybrid games (C games must set an update callback), and `pdz`/`pdi`
  compiled assets (games run from their source folder with PNG images and
  `.fnt` fonts).
