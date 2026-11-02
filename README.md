# Playdate Performance Simulator

Run a Playdate game on your computer **at device speed** and see where the
frame time goes, before you put it on hardware.

Panic's Playdate Simulator runs games at full desktop speed, so performance
problems only show up on the device (168 MHz Cortex-M7, 16 MB RAM, 400×240
1-bit display). This simulator charges every piece of work the game does to a
**virtual device clock** using a cost model calibrated on a real Playdate, and
shows live performance data while the game runs.

> Status: under active development. See [`docs/PLAN.md`](docs/PLAN.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

This project is not affiliated with Panic Inc. The Playdate SDK is not
included and must never be committed to this repository.
