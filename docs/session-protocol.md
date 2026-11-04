# Session protocol

`pdsim-session` runs one game on its own thread. A front end sends it
commands and receives events, both JSON objects with a `type` field. The
Tauri app carries them over IPC (`invoke("send", {cmd})` and the `pdsim`
event); `pdsim serve` carries them over a WebSocket, one text message per
command or event. The same frontend (`app/src`) works with either; in a
browser it connects to `ws://<host>:8765`, or to the URL in `?ws=`.

## Commands

| `type` | Fields | Effect |
|---|---|---|
| `load` | `path`, `calibration?` | Load or reload a game (any path `pdsim run` accepts) with an optional profile path, and start running. |
| `play` / `pause` | | Run or stop the frame loop. |
| `step` | | Pause, then run exactly one frame and send its `flame`. |
| `setMode` | `mode`: `"device"` or `"turbo"` | Device-speed pacing or as fast as possible. |
| `setSpeed` | `speed` | Multiplier on device-speed pacing (clamped to 0.05–20). |
| `button` | `name`, `down` | Press or release `a`, `b`, `up`, `down`, `left`, `right`. |
| `crank` | `delta` | Turn the crank by `delta` degrees. |
| `crankTo` | `angle` | Set the crank angle. |
| `dock` | `docked` | Dock or undock the crank. |
| `accel` | `x`, `y`, `z` | Set the accelerometer. |
| `breakOnOverBudget` | `on` | Pause after any over-budget frame. |
| `exportTrace` | `path?` | Write a Chrome trace of the last 3000 frames (default `~/pdsim-trace-<frame>.json`). |
| `flame` | `index?` | Request the flame data of a recorded frame (latest if omitted). |
| `hello` | | A UI (re)connected: resend `loaded` and `state`. |
| `quit` | | Stop the session thread. |

## Events

| `type` | Fields | When |
|---|---|---|
| `loaded` | `path`, `description`, `calibration`, `estimated`, `cpuHz`, `ramBytes`, `categories` | A game loaded. |
| `state` | `running`, `mode`, `speed`, `breakOnOverBudget`, `loaded` | After any command that changes them. |
| `frame` | `index`, `fb`, `rows`, `inverted`, `overBudget` | Every frame. `fb` is the 400×240 1-bit framebuffer, base64, 52-byte rows (bit 7 of byte 0 is pixel 0; 1 = white). `rows` is a base64 240-bit mask of rows redrawn this frame (bit *i* % 8 of byte *i* / 8). |
| `stats` | `frames`, `hotspots`, `droppedTotal`, `overBudgetTotal`, `frameIndex`, `deviceMs`, `refreshRate`, `memory`, `realtime` | About 10×/s and on pause. `frames` holds every frame since the last `stats`: `{index, workMs, frameMs, budgetMs, cpu, categories, overBudget, dropped, rows, memory}`. `hotspots` is the top 40. `realtime` is device time ÷ wall time since the previous `stats` (about 1 at device speed, more in turbo, less when the host can't keep up), or `null` when paused or stepping. |
| `flame` | `index`, `workCycles`, `folded` | After `step`, a break, or a `flame` command. `folded` maps `;`-separated stacks to cycles. |
| `log` | `lines` | New console output. |
| `break` | `reason` | Paused by break-on-over-budget. |
| `traceExported` | `path`, `frames` | `exportTrace` succeeded. |
| `error` | `message` | Load failed, a game error (the session pauses), or a bad calibration path. |

## Pacing

In device mode the session sleeps so that wall-clock time follows device
time (scaled by `speed`). Commands that arrive during the sleep are handled
at once, so input is not delayed by a frame. If the host falls more than
250 ms behind (a slow host or a very heavy frame), the session re-anchors
instead of rushing to catch up.

## Example

```bash
cargo run --release -p pdsim-cli -- serve examples/lua-stress
```

```js
const ws = new WebSocket("ws://127.0.0.1:8765");
ws.onopen = () => ws.send(JSON.stringify({ type: "setMode", mode: "turbo" }));
ws.onmessage = (m) => {
  const e = JSON.parse(m.data);
  if (e.type === "stats") console.log(e.frameIndex, e.overBudgetTotal);
};
```
