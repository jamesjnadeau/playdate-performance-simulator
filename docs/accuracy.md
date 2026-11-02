# Accuracy notes

## Status

The built-in profile (`calibration/default.json`) is **estimated**, not
measured. No device-fitted profile ships yet; the target once calibrated is
average frame time within about 15% of the device per game. Please add rows
below when you calibrate (see [calibration.md](calibration.md)).

| Game | Device | Profile | Avg frame (device) | Avg frame (sim) | Error |
|---|---|---|---|---|---|
| examples/lua-stress | – | estimates | – | 11.6 ms | – |
| examples/c-stress | – | estimates | – | 3.2 ms | – |

To compare: run the game on the device with `playdate.getStats()` /
`playdate.getElapsedTime()` around `playdate.update`, and run the same frames
in `pdsim run --frames-out` with the fitted profile.

## What is modeled

- **Lua**: every opcode, at a per-opcode cost; C library calls at an average
  cost; allocation/free volume and incremental GC work. The VM and GC are the
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
- The Lua `[C]` library function cost is one average (`string.format` and
  `math.abs` cost the same).
- Graphics run the simulator's own rasterizers. Pixel output matches the
  device for the common cases (see the golden tests), but some details
  (ellipse edge pixels, text kerning edge cases, `drawSampled`, blur) are
  approximations, and rotated/scaled drawing uses nearest-neighbour sampling.
- Not simulated: video, networking, scoreboards, the system menu, Lua+C
  hybrid games (C games must set an update callback), and `pdz`/`pdi`
  compiled assets (games run from their source folder with PNG images and
  `.fnt` fonts).
