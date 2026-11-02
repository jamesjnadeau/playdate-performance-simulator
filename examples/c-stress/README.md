# c-stress

A C sample with known hot paths, used to validate the ARM backend:

- `update_particles`: float physics for 400 particles
- `draw_particles`: 400 `fillRect` calls per frame
- `spike`: sorts 40,000 integers with `qsort` every 60 frames (over budget)
- 8 sprites with update callbacks

Build the device binary (needs the SDK and `arm-none-eabi-gcc`), then run it:

```bash
make -C examples/c-stress device_bin
cargo run -p pdsim-cli -- run examples/c-stress --frames 300
```
