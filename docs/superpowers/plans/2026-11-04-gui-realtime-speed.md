# Optimised Dev Builds and Real-Time Speed Tile Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make dev builds (`tauri dev`, `cargo run`, `cargo test`) simulate at near-release speed, and show in the GUI how fast the simulation runs compared with real time.

**Architecture:** A workspace `Cargo.toml` profile change optimises dependencies (Unicorn included) fully and pdsim's own crates lightly in dev builds. `pdsim-session` measures device ms ÷ wall ms between `stats` events and adds it as `realtime`. The web UI shows it in a new "Speed" tile.

**Tech Stack:** Rust (Cargo profiles, `pdsim-session`), TypeScript + Vite frontend (`app/src`), Tauri v2.

**Spec:** `docs/superpowers/specs/2026-11-04-gui-realtime-speed-design.md`

## Global Constraints

- Dependencies: `[profile.dev.package."*"] opt-level = 3`. Workspace crates: `[profile.dev] opt-level = 1`. Debug info and debug assertions stay on.
- Success for the profile change: the debug CrankSail sailing run takes ≤ about 1.5× the release time (now 180 s vs 20.2 s).
- `realtime` is `null` when not running (paused, after a step) or when < 50 ms of wall time passed since the last `stats`.
- `realtime` is reported in Turbo too.
- Warning colour only in Device mode, when the smoothed value is < 0.9× (scaled by the `speed` setting).
- Tooltip text (exact): "How fast the simulation runs compared with the device. Below 1× the game is in slow motion because the host can't keep up; device timings are still correct."
- No changes to how frames are sent or drawn. No emulator hot-path work.

## Review Focus

1. **Pacing re-anchor:** `pace()` calls `reanchor()` when the host falls more than 250 ms behind. `realtime` must use its own baseline, not the pacing anchor, or a slow host would read about 1×. (Task 2's `realtime_reflects_host_speed`, via its own fields.)
2. **Speed multiplier:** with `setSpeed 0.5`, device mode at full speed reads 0.5×. That isn't "falling behind" and must not warn. (Task 3: the threshold compares against `0.9 × speed`.)
3. **Load / mode switch:** the smoothing buffer must reset on `loaded` and on mode change, so stale turbo values (for example 40×) don't sit in a device-mode average. (Task 3.)
4. **Step / pause:** a `stats` sent on pause or step carries `null`, and the tile shows `–`, not a stale number. (Task 2's test asserts the null; Task 3 handles it.)
5. **Hot crates still too slow at opt-level 1:** if the debug run misses the 1.5× target, raise the five hot crates to 2 rather than raising everything. (Task 1, Step 4.)

---

### Task 1: Optimise dev builds

**Files:**
- Modify: `Cargo.toml` (append profiles)

**Interfaces:** none.

- [ ] **Step 1: Record the baseline** (debug build already exists from investigation; rebuild if missing with `cargo build -p pdsim-cli`)

Run, with `G=/home/user/jamesjnadeau/playdate-sailing-sim`, `PLAYDATE_SDK_PATH=/root/PlaydateSDK`, after `make device_bin` in `$G` (already built):
`time target/debug/pdsim run $G/Source --frames 600 --input $G/tools/perf_free_sail.json -q`
Expected: about 180 s, with `work avg 24.83 ms` in the output. Note the release time too (`target/release/pdsim`, about 20 s).

- [ ] **Step 2: Add the profiles to `Cargo.toml`**

```toml
# Dev builds run games: optimise dependencies (Unicorn's C build follows
# opt-level via cmake) fully, and our own crates lightly so rebuilds stay quick.
[profile.dev]
opt-level = 1

[profile.dev.package."*"]
opt-level = 3
```

- [ ] **Step 3: Rebuild and re-time**

Run: `cargo build -p pdsim-cli`, then the Step 1 command.
Expected: ≤ about 30 s (1.5× release), with output identical to the baseline (`work avg 24.83 ms`, same hotspots). Device timings must not change.

- [ ] **Step 4: Only if Step 3 misses the target**, add `[profile.dev.package.<name>] opt-level = 2` for `pdsim-arm`, `pdsim-lua`, `pdsim-core`, `pdsim-gfx` and `pdsim-device`, then re-time. Record the final numbers for the commit message.

- [ ] **Step 5: Run the workspace tests**

Run: `cargo test --workspace`
Expected: all pass.

- [ ] **Step 6: Commit** (`Cargo.toml`), with the before/after timings in the message.

---

### Task 2: `realtime` in session `stats`

**Files:**
- Modify: `crates/pdsim-session/src/lib.rs` (Runner fields, `reanchor`, `flush_stats`, tests)
- Modify: `docs/session-protocol.md` (the `stats` row)

**Interfaces:**
- Produces: the `stats` event field `realtime: number | null`. `realtime = Δdevice_ms / Δwall_ms` since the previous `stats` or (re)anchor.
- Produces (private): `fn realtime_ratio(device_ms: f64, wall: Duration) -> Option<f64>`, which returns `None` when `wall < 50 ms`.

- [ ] **Step 1: Write the failing tests** in the existing `mod tests`

```rust
#[test]
fn realtime_ratio_needs_enough_wall_time() {
    assert_eq!(realtime_ratio(100.0, Duration::from_millis(40)), None);
    assert_eq!(realtime_ratio(100.0, Duration::from_millis(100)), Some(1.0));
    assert_eq!(realtime_ratio(50.0, Duration::from_millis(100)), Some(0.5));
}

#[test]
fn stats_report_realtime() {
    // load lua-stress (example()), wait for "loaded", sleep 600 ms in device mode
    // -> the latest stats with non-null realtime: 0.7 <= r <= 1.3
    // clear; SetMode turbo; sleep 600 ms -> latest non-null realtime > 1.5
    // clear; Pause; wait_for "stats" -> realtime is null
}
```

Write the second test fully, in the style of `device_speed_runs_in_real_time_and_turbo_is_faster` (same `Arc<Mutex<Vec<Value>>>` collector and `wait_for`). "Latest non-null" means filtering `type == "stats"` and `realtime.is_f64()`, then taking the last.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p pdsim-session realtime`
Expected: compile error (`realtime_ratio` not found).

- [ ] **Step 3: Implement**
- Add Runner fields `rt_wall: Instant` and `rt_device_ms: f64`. They are separate from `anchor_*`, because `pace()` re-anchors on lag (Review Focus 1).
- Add `fn mark_realtime(&mut self)`, which sets them to now / the scheduler's `now_ms()`. Call it from `reanchor()`. Do **not** reset it from `pace()`'s lag branch: `reanchor()` is called there too, so give that branch a pacing-only re-anchor (split `reanchor` into `reanchor_pacing` plus `mark_realtime`, and have `pace()` call only `reanchor_pacing`).
- In `flush_stats`: `realtime = if self.running { realtime_ratio(now_ms - rt_device_ms, rt_wall.elapsed()) } else { None }`, then `mark_realtime()`. Emit as `"realtime": realtime`, which serialises to `null` for `None`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pdsim-session`
Expected: all pass, including the two existing tests.

- [ ] **Step 5: Document it.** In `docs/session-protocol.md`, add `realtime` to the `stats` field list and append: "`realtime` is device time ÷ wall time since the previous `stats` (about 1 at device speed, more in turbo, less when the host can't keep up), or `null` when paused or stepping."

- [ ] **Step 6: Commit** (`lib.rs`, `session-protocol.md`).

---

### Task 3: Speed tile in the GUI

**Files:**
- Modify: `app/index.html` (new tile after the `tFps` tile)
- Modify: `app/src/main.ts` (smoothing and rendering)
- Modify: `app/src/style.css` (`.tiles` columns 6 → 7)

**Interfaces:**
- Consumes: `stats.realtime` (Task 2); `state.mode` and `state.speed` (existing).

- [ ] **Step 1: Add the tile markup:** `<div class="tile" title="<exact tooltip from Global Constraints>"><span class="label">Speed</span><span id="tSpeed" class="value">–</span></div>`. Change `.tiles` to `repeat(7, minmax(0, 1fr))`.

- [ ] **Step 2: Implement in `main.ts`**
- Module state: `rtSamples: number[]` (last 5), `mode = "device"`, `speed = 1`.
- In `state`: store `e.mode` and `e.speed`. If the mode changed, clear `rtSamples`.
- In `loaded`: clear `rtSamples`.
- In `stats`: if `e.realtime == null`, clear `rtSamples` and show `–`. Otherwise push, keep the last 5, and average.
- Text: `${avg < 10 ? avg.toFixed(2) : avg.toFixed(0)}× real time`.
- Toggle `.bad` when `mode === "device" && avg < 0.9 * speed`.

- [ ] **Step 3: Type-check and build**

Run: `cd app && npm run build`
Expected: no errors.

- [ ] **Step 4: Verify in a browser** (throwaway script in the scratchpad, not committed). Run `target/release/pdsim serve examples/lua-stress` plus `npx vite` in `app/`. Playwright loads `http://127.0.0.1:1420/?ws=ws://127.0.0.1:8765`.
- Device mode, after about 3 s: `#tSpeed` reads `0.9x`–`1.1x` and has no `.bad`.
- Switch `#mode` to turbo: the value is > 1, with no `.bad`.
- Slow host: build an unoptimised CLI into a separate target dir:
  `cargo build -p pdsim-cli --target-dir <scratchpad>/slow --config 'profile.dev.opt-level=0' --config 'profile.dev.package."*".opt-level=0'`.
  Serve the sailing game with it and press A twice to start sailing. Expected: about 0.1×, with `.bad`.
- Fixed dev build: serve the sailing game with `target/debug/pdsim` (Task 1's profile). Expected: about 1×, which confirms the fix end to end.

- [ ] **Step 5: Commit** (`index.html`, `main.ts`, `style.css`).
