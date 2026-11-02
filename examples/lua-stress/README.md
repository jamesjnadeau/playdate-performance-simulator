# lua-stress

A sample game with known hot paths, used to validate the simulator:

- `Ball:update`: per-sprite movement (20 × `work` sprites)
- `physics`: a float-math loop (1500 × `work` iterations per frame)
- `spike`: a 40,000-table allocation burst every `spikeEvery` frames, which
  should show up as an over-budget frame with GC time

Put `{"work": 2, "spikeEvery": 60}` in `config.json` in the data folder to
change the workload.

```bash
cargo run -p pdsim-cli -- run examples/lua-stress --frames 300 --report r.json
```
