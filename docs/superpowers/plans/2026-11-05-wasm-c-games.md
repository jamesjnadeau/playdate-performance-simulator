# C Games in the Browser (WASM) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run C Playdate games (`pdex.elf`) in the browser by putting a CPU trait in front of Unicorn and adding a pure-Rust Thumb-2 interpreter that gives byte-identical reports.

**Architecture:** `pdsim-arm` stops naming `Unicorn` and talks to a `trait Cpu` (memory, registers, `emu_start`/`emu_stop`, plus two hooks: `on_block` and `on_trampoline`). Unicorn implements it behind the `unicorn` feature. A new `pdsim-thumb` crate implements it with a block-cached ARMv7E-M + FPv5 interpreter, behind the `thumb` feature. A lockstep harness runs both natively and reports the first divergence. Once reports match, `pdsim-wasm` turns on `c-games` with `thumb` only, and the browser prompts for the SDK, which C games need for `pd_api.h`.

**Tech Stack:** Rust (stable), unicorn-engine 2.1 (native only), capstone 0.14 (costing; builds for Emscripten), Emscripten, `arm-none-eabi-gcc` (to build test ELFs; CI already has it for `examples/c-stress`).

**Spec:** [docs/wasm-port.md](../../wasm-port.md), phases 3–5. Depends on the Lua plan, [2026-11-05-wasm-lua-browser.md](2026-11-05-wasm-lua-browser.md), for Task 5 only (the `pdsim-wasm` crate, the `c-games` feature and the browser SDK loader). Tasks 1–4 can start right away.

## Global Constraints

- **Same numbers:** "`report.json` from native must equal `report.json` from WASM (compare cycles exactly, not within a tolerance)". Here that means `pdsim-thumb` and Unicorn reports must be byte-identical.
- "**Block boundaries must match** what Unicorn/Capstone produce today, so that per-block costs, and so cycle totals, stay identical." Keep Capstone for costing.
- Scope is only what a `pdex.elf` runs in user code: ARMv7E-M Thumb/Thumb-2 including DSP, plus FPv5 single precision. Double precision is added only if `arm-none-eabi-objdump -d` of the example and calibration ELFs shows `.f64` instructions. There are no interrupts, MPU or privilege levels.
- "Unknown instructions fail with the same crash message format as `crash_message` today." An unmapped access is a crash, as in Unicorn.
- IEEE results must be bit-exact. FPSCR rounding is round-to-nearest, and flush-to-zero follows FPSCR.FZ.
- Native builds keep working at every commit. `cargo test --workspace` and clippy `-D warnings` pass with default features (`unicorn`).
- **No SDK files in the bundle or the repo.** The browser reads `pd_api.h` from the user's SDK folder. Never commit SDK files, and don't commit a generated layout table (the spec defers that pending a license check).
- The GPL note in README applies to builds with `unicorn`. A `thumb`-only build doesn't link Unicorn. Say so in the README.

## Review Focus

1. **A C API callback that runs game code again** (for example a sprite update or draw function called from `sprite->updateAndDrawSprites`, i.e. nested `emu_start` inside `on_trampoline`): the inner call returns to the trampoline, and the outer call resumes with its registers intact on both backends. Test: Task 1, `nested_call_from_trampoline_restores_the_outer_call`.
2. **A conditional branch or `IT` block at the end of a block, and a block that runs straight into the next one with no branch:** block boundaries, and so the number of `on_block` calls and their addresses, match Unicorn exactly. Test: Task 3, `block_trace_matches_unicorn` on every ELF.
3. **Game code that hits an unsupported instruction or reads unmapped memory** (a null-pointer read in user code): the session reports the same crash message as Unicorn instead of panicking or looping. Test: Task 2, `unknown_instruction_crashes_like_unicorn` and `unmapped_read_is_a_crash`.
4. **Floating-point edge cases games actually hit** (NaN inputs, `VCVT` of an out-of-range or negative float to an unsigned int, denormals under FZ): bit-exact Unicorn results. Tests: Task 2, `vfp_nan_and_default_nan`, `vcvt_saturates_and_rounds`, `fz_flushes_denormals`.
5. **A browser user who opens a C game before loading the SDK:** a clear prompt to load it, not a crash or a generic error, and the game loads once the SDK is given. Test: Task 5, `c_game_without_sdk_names_the_sdk` plus the manual check.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/pdsim-cpu/src/lib.rs` (create) | `trait Cpu<S>`, `Reg`, `CpuError`, `Hooks<S>` (the two callbacks); no dependencies |
| `crates/pdsim-arm/src/cpu.rs` (create) | Re-exports `pdsim_cpu`, defines `Backend` |
| `crates/pdsim-arm/src/unicorn_cpu.rs` (create) | `UnicornCpu: Cpu` (feature `unicorn`) |
| `crates/pdsim-arm/src/{emu,api,abi}.rs` (modify) | Use `&mut dyn Cpu<ArmState>` + `&mut ArmState` instead of `Unicorn<ArmState>` |
| `crates/pdsim-arm/Cargo.toml` (modify) | features `unicorn` (default) and `thumb`, both optional deps |
| `crates/pdsim-thumb/src/lib.rs` (create) | `ThumbCpu: Cpu`, run loop, block cache |
| `crates/pdsim-thumb/src/mem.rs` | Flat regions, bounds-checked little-endian access, write log for the harness |
| `crates/pdsim-thumb/src/decode.rs` | Thumb/Thumb-2 decode to `Op` and block splitting |
| `crates/pdsim-thumb/src/exec.rs` | Integer, DSP and flag semantics |
| `crates/pdsim-thumb/src/vfp.rs` | FPv5 single precision, FPSCR |
| `crates/pdsim-thumb/tests/insn.rs` | Per-instruction unit tests (hand-assembled halfwords) |
| `crates/pdsim-arm/tests/lockstep.rs` (create) | Differential harness (needs both features) |
| `tests/thumb-corpus/` (create) | Small C test functions, `Makefile` that builds `corpus.elf` with `arm-none-eabi-gcc`; the ELF is a build product, not committed |
| `scripts/thumb-parity.sh` (create) | Reports from Unicorn vs `pdsim-thumb` for every example and calibration ELF |

---

### Task 1: `trait Cpu` in front of Unicorn

**Files:**
- Create: `crates/pdsim-cpu/{Cargo.toml,src/lib.rs}`, `crates/pdsim-arm/src/cpu.rs`, `crates/pdsim-arm/src/unicorn_cpu.rs`
- Modify: `crates/pdsim-arm/src/{lib,emu,api,abi,layout}.rs`, `crates/pdsim-arm/Cargo.toml`, `crates/pdsim-runner/Cargo.toml` (`c-games = ["dep:pdsim-arm", "pdsim-arm/unicorn"]`)
- Test: `crates/pdsim-arm/tests/c_game.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: a new dependency-free crate `crates/pdsim-cpu` (so `pdsim-thumb` can implement the trait without depending on `pdsim-arm`), re-exported as `pdsim_arm::cpu`, and used as `dyn Cpu<ArmState>` in `pdsim-arm`:
  ```rust
  pub enum Reg { R(u8) /* 0..=12 */, Sp, Lr, Pc, S(u8) /* 0..=31 */, Fpscr, Cpsr }
  pub struct CpuError { pub kind: String, pub pc: u32 }   // kind matches today's crash_message wording
  pub trait Cpu<S> {
      fn mem_map(&mut self, addr: u32, size: u32, writable: bool) -> Result<(), CpuError>;
      fn mem_read(&self, addr: u32, buf: &mut [u8]) -> Result<(), CpuError>;
      fn mem_write(&mut self, addr: u32, data: &[u8]) -> Result<(), CpuError>;
      fn reg_read(&self, r: Reg) -> u32;
      fn reg_write(&mut self, r: Reg, v: u32);
      /// Runs from `begin` (Thumb bit set) until PC == `until` or `emu_stop`. Re-entrant: may be
      /// called from inside `Hooks::on_trampoline`.
      fn emu_start(&mut self, begin: u32, until: u32, state: &mut S, hooks: &dyn Hooks<S>) -> Result<(), CpuError>;
      fn emu_stop(&mut self);
  }
  pub trait Hooks<S> {
      fn on_block(&self, cpu: &mut dyn Cpu<S>, state: &mut S, addr: u32, size: u32);
      fn on_trampoline(&self, cpu: &mut dyn Cpu<S>, state: &mut S, addr: u32);
  }
  ```
  - `ArmGame` owns `Box<dyn Cpu<ArmState>>` and `ArmState` separately, so no `get_data()`.
  - `call`, `call_full`, `set_ret` and `Regs` take `(&mut dyn Cpu<ArmState>, &mut ArmState)`.
  - `ArmGame::emulator()` becomes `cpu(&mut self) -> &mut dyn Cpu<ArmState>`.
  - `ArmGame::load(…, backend: Backend)` with `pub enum Backend { #[cfg(feature="unicorn")] Unicorn, #[cfg(feature="thumb")] Thumb }` and `Backend::default()` (Unicorn when available).
  - `UnicornCpu::new() -> Result<UnicornCpu, CpuError>` sets the Cortex-M7 model. Its block and code hooks forward to `Hooks` through a raw pointer held for the duration of `emu_start`, which is the only `unsafe` in the change.

- [ ] **Step 1: Write the failing test** (`crates/pdsim-arm/tests/c_game.rs`, gated on the ELF existing like the other tests there)

```rust
#[test]
fn nested_call_from_trampoline_restores_the_outer_call() {
    // examples/c-stress registers a sprite update function; run 5 frames through the trait and
    // compare the per-frame cycle totals with the values recorded from main before this change.
    let mut g = load_c_stress(Backend::default());
    let cycles: Vec<u64> = (0..5).map(|_| g.frame(None).unwrap().cycles).collect();
    assert_eq!(cycles, C_STRESS_FIRST_5_FRAMES);
}
```

Record `C_STRESS_FIRST_5_FRAMES` by running the test body once on `main` before refactoring, and paste the values in as a `const`.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p pdsim-arm --test c_game nested_call`
Expected: FAIL to compile (`Backend` not defined).

- [ ] **Step 3: Implement** `cpu.rs` and `unicorn_cpu.rs`, and move `emu.rs`/`api.rs`/`abi.rs` to the trait. About 126 `uc.` call sites in `api.rs` change mechanically: `uc.mem_read` becomes `cpu.mem_read`, and `uc.get_data_mut()` becomes `state`. `Mem` (emu.rs:222) is implemented for `dyn Cpu<ArmState>`.

- [ ] **Step 4: Run tests and check the report is unchanged**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`, then `make -C examples/c-stress device_bin && cargo run --release -p pdsim-cli -- run examples/c-stress --frames 300 -q --report /tmp/after.json` and compare it with a report from `main` (`git stash`, same command, `--report /tmp/before.json`) using `cmp`.
Expected: PASS, and `cmp` prints nothing.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/pdsim-cpu crates/pdsim-arm crates/pdsim-runner/Cargo.toml
git commit -m "Put a Cpu trait in front of Unicorn"
```

---

### Task 2: `pdsim-thumb`: memory, decoder, integer, DSP and VFP execution

**Files:**
- Create: `crates/pdsim-thumb/{Cargo.toml, src/lib.rs, src/mem.rs, src/decode.rs, src/exec.rs, src/vfp.rs, tests/insn.rs}`
- Modify: `Cargo.toml` (workspace member), `crates/pdsim-arm/Cargo.toml` (`thumb = ["dep:pdsim-thumb"]`), `crates/pdsim-arm/src/cpu.rs` (`Backend::Thumb` builds `pdsim_thumb::ThumbCpu`)

**Interfaces:**
- Consumes: `pdsim_cpu::{Cpu<S>, Hooks<S>, Reg, CpuError}` (Task 1). `pdsim-thumb` depends on `pdsim-cpu` only, never on `pdsim-arm`.
- Produces:
  - `pdsim_thumb::ThumbCpu::new() -> ThumbCpu`, which implements `Cpu`.
  - `ThumbCpu::write_log(&mut self) -> Vec<(u32, Vec<u8>)>` drains the write log; the log is only recorded after `set_write_log(true)`.
  - `decode::Op` (internal) and `decode::block_at(mem, addr) -> Result<Block, CpuError>`, with `Block { ops: Vec<Op>, size: u32 }`. A block ends after the first branch, PC write, `IT`-conditional branch, `TBB`/`TBH`, `SVC`/`BKPT`, or at the trampoline range. This is the same rule as QEMU's TB end, which Task 3 checks.
  - The block cache is `Vec<Option<Block>>` indexed by `(addr - code_base) / 2`, with a write guard: a write into the code region clears the cache.
  - Crash messages: unknown encodings give `CpuError { kind: "invalid instruction", pc }`, and unmapped accesses give `"unmapped read"` / `"unmapped write"` with the address. Check the wording against `crash_message` (emu.rs:388) and reuse its strings.

- [ ] **Step 1: Write the failing tests** (`tests/insn.rs`). Each test maps a code page, writes halfwords and runs one block. The helper is `fn run(code: &[u16], setup: impl FnOnce(&mut ThumbCpu)) -> ThumbCpu`.

```rust
#[test] fn adds_sets_carry_and_overflow()            // ADDS r0, r0, r1 with 0x7fffffff + 1: N=1 V=1 C=0
#[test] fn adc_sbc_use_and_set_carry()               // SBCS with C=0 subtracts one more; ADCS 0xffffffff+0+C=1 -> 0, C=1
#[test] fn shifted_register_carry_out()              // ANDS r0, r1, r2, LSR #32 -> C = bit 31 of r2
#[test] fn it_block_with_flag_setting_inside()       // ITTE EQ; ADDS (sets flags inside the block); MOV; MOV - the E slot uses the original EQ
#[test] fn ssat_usat_set_q()                         // SSAT r0, #8, r1 with 300 -> 127 and APSR.Q set; USAT with -5 -> 0, Q set
#[test] fn umull_smlal_sdiv_udiv()                   // incl. SDIV by zero -> 0 (DIV_0_TRP clear) and INT_MIN / -1 -> INT_MIN
#[test] fn ldm_stm_push_pop_writeback()              // POP {r4, pc} ends the block and sets PC with Thumb bit cleared
#[test] fn tbb_tbh_branch()                          // table jump to the 2nd entry
#[test] fn simd_uadd8_and_sel()                      // GE flags set per byte, SEL picks bytes by GE
#[test] fn vfp_nan_and_default_nan()                 // VADD.F32 with qNaN operand propagates it; with FPSCR.DN=1 gives 0x7fc00000
#[test] fn vcvt_saturates_and_rounds()               // VCVT.U32.F32 -1.5 -> 0, 5e9 -> 0xffffffff; VCVTR.S32.F32 2.5 -> 2 (ties to even)
#[test] fn fz_flushes_denormals()                    // FPSCR.FZ=1: VMUL.F32 of a denormal input gives +0
#[test] fn vcmp_vmrs_sets_apsr()                     // VCMP.F32 s0, s1 with NaN -> NZCV = 0011
#[test] fn unknown_instruction_crashes_like_unicorn() // 0xde00 (UDF) -> Err kind == "invalid instruction", pc == code base
#[test] fn unmapped_read_is_a_crash()                // LDR r0, [r1] with r1 = 0 -> Err kind starts with "unmapped read"
```

The expected values come from the Armv7-M ARM pseudocode. Where in doubt, confirm them by running the same halfwords on `UnicornCpu` in a scratch test.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p pdsim-thumb`
Expected: FAIL to compile.

- [ ] **Step 3: Implement** the four modules, in this order, with a commit after each group whose tests pass: `mem` → integer data processing and flags → loads/stores, branches and `IT` → multiply/divide/bitfield → DSP and SIMD → VFP. Use one `match` on the top 5 bits of the first halfword, then sub-decoders per encoding table section of the Armv7-M ARM (A5.2–A5.3, A7.7 and the FP section). Flags live as four `bool`s plus `q` and a `ge: u8`, packed only on `reg_read(Reg::Cpsr)`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p pdsim-thumb && cargo clippy -p pdsim-thumb --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/pdsim-thumb crates/pdsim-arm
git commit -m "Add pdsim-thumb: a Thumb-2 + FPv5 interpreter behind the Cpu trait"
```

---

### Task 3: Lockstep differential harness and a generated test corpus

**Files:**
- Create: `crates/pdsim-arm/tests/lockstep.rs`, `tests/thumb-corpus/{Makefile, corpus.c, gen.py}`
- Modify: `crates/pdsim-thumb/src/*` (fixes the harness turns up), `.gitignore` (`tests/thumb-corpus/*.elf`, `tests/thumb-corpus/gen_*.c`)

**Interfaces:**
- Consumes: `UnicornCpu`, `ThumbCpu`, `ThumbCpu::write_log` (Tasks 1–2).
- Produces:
  - `fn lockstep(elf: &Path, frames: u32) -> Result<(), Divergence>` (test-only), where `Divergence { block: u32, pc: u32, reg: Option<(Reg, u32, u32)>, write: Option<(u32, Vec<u8>, Vec<u8>)>, disasm: String }`. Its `Display` shows the block index, the PC, the differing register or write (Unicorn vs thumb), and ten instructions of Capstone disassembly around the PC.
  - Runs are deterministic, so "lockstep" is done by recording and comparing, with no interleaving. Load two independent `ArmGame`s from the same ELF, one per backend, each with its own `Device`. A test-only `ArmGame::set_block_probe(Box<dyn FnMut(&dyn Cpu<ArmState>, u32)>)` snapshots every `Reg` plus the write log drained since the previous block at each `on_block`. Run both one frame at a time and compare snapshot vectors. The first mismatch is the `Divergence`. On Unicorn, the write log comes from a `UC_HOOK_MEM_WRITE` hook that is installed only while a probe is set.
  - `block_trace(elf, frames, backend) -> Vec<(u32, u32)>` records `(addr, size)` for each `on_block`.
  - `tests/thumb-corpus/gen.py --seed N --count 200` writes `gen_N.c`: random straight-line functions per encoding class (flag-setting ALU with random immediates and shifts, `IT` patterns, DSP saturation, `VCVT`/`VCMP` on special floats from a fixed table), each called from `eventHandler` once per frame. `make` builds `corpus.elf` the same way `examples/c-stress` builds its device binary.

- [ ] **Step 1: Write the failing tests**

```rust
// lockstep.rs; every test returns early with a printed skip when its ELF hasn't been built
#[test] fn lockstep_c_stress()            { lockstep(&elf("examples/c-stress"), 120).unwrap() }
#[test] fn lockstep_bench_c()             { lockstep(&elf("calibration/bench-c"), 60).unwrap() }
#[test] fn lockstep_corpus()              { for seed in 0..4 { lockstep(&corpus_elf(seed), 3).unwrap() } }
#[test] fn block_trace_matches_unicorn()  { for e in all_elfs() { assert_eq!(block_trace(&e, 30, Unicorn), block_trace(&e, 30, Thumb)) } }
```

- [ ] **Step 2: Build the ELFs and run the tests to see them fail**

Run: `make -C examples/c-stress device_bin && make -C calibration/bench-c device_bin && make -C tests/thumb-corpus SEEDS="0 1 2 3" && cargo test -p pdsim-arm --features unicorn,thumb --test lockstep`
Expected: FAIL with a `Divergence` (or missing-instruction crashes), each naming the first bad instruction.

- [ ] **Step 3: Fix divergences** in `pdsim-thumb` until all four tests pass. For each fix, add a regression case to `tests/insn.rs` with the instruction from the `Divergence` output. If `arm-none-eabi-objdump -d` shows `.f64` instructions in any ELF, add double precision to `vfp.rs` here.

- [ ] **Step 4: Run tests**

Run: `cargo test -p pdsim-arm --features unicorn,thumb --test lockstep && cargo test -p pdsim-thumb`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/pdsim-arm/tests/lockstep.rs crates/pdsim-thumb tests/thumb-corpus .gitignore
git commit -m "Check pdsim-thumb against Unicorn in lockstep on every ELF and a generated corpus"
```

---

### Task 4: Report parity and a `--cpu` switch

**Files:**
- Create: `scripts/thumb-parity.sh`
- Modify: `crates/pdsim-runner/src/lib.rs` (`RunConfig::cpu: CpuBackend`), `crates/pdsim-runner/Cargo.toml` (feature `thumb = ["pdsim-arm/thumb"]`), `crates/pdsim-cli/src/main.rs` and `Cargo.toml` (`--cpu unicorn|thumb`, forwarded `thumb` feature), `.github/workflows/ci.yml`, `docs/cli.md`, `README.md`
- Test: `crates/pdsim-cli/tests/cli.rs`

**Interfaces:**
- Consumes: `pdsim_arm::Backend` (Task 1), `ThumbCpu` (Task 2).
- Produces:
  - `pdsim_runner::CpuBackend` re-exports `pdsim_arm::Backend` (it exists when `c-games` is on); `RunConfig { cpu: CpuBackend, .. }` defaults to `Backend::default()`.
  - CLI: `--cpu unicorn|thumb` (default: unicorn when built in, otherwise thumb). Picking a backend that isn't compiled in fails with `pdsim: this build has no <name> CPU (rebuild with --features <name>)`, exit code 2.
  - `scripts/thumb-parity.sh` builds `pdsim-cli --features thumb`, runs `examples/c-stress` and `calibration/bench-c` for 300 frames with `--cpu unicorn` and `--cpu thumb`, and `cmp`s the reports. It prints `thumb parity OK: 2 projects` on success.
  - The CI `c-games` job (the one that already builds `examples/c-stress`) also runs the lockstep tests and `scripts/thumb-parity.sh`.

- [ ] **Step 1: Write the failing test** (`cli.rs`, skipped if `examples/c-stress/pdex.elf` is missing)

```rust
#[test]
fn thumb_and_unicorn_reports_are_identical() {
    let a = run_c_stress(&["--cpu", "unicorn"]);
    let b = run_c_stress(&["--cpu", "thumb"]);
    assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p pdsim-cli --features thumb --test cli thumb_and_unicorn`
Expected: FAIL (unknown argument `--cpu`).

- [ ] **Step 3: Implement** the backend option, the CLI flag, the script, the CI step, and docs: `--cpu` in `docs/cli.md`, and a README note that a `--no-default-features --features thumb` build has no Unicorn and so no GPL obligation.

- [ ] **Step 4: Run tests and parity**

Run: `cargo test --workspace --features pdsim-cli/thumb && scripts/thumb-parity.sh`
Expected: PASS, `thumb parity OK: 2 projects`.

- [ ] **Step 5: Commit**

```bash
git add crates scripts/thumb-parity.sh .github/workflows/ci.yml docs/cli.md README.md
git commit -m "Add --cpu thumb and check report parity with Unicorn in CI"
```

---

### Task 5: C games in the browser

**Files:**
- Modify: `crates/pdsim-wasm/Cargo.toml` (`pdsim-session` with `default-features = false, features = ["c-games-thumb"]`), `crates/pdsim-session/Cargo.toml` and `crates/pdsim-runner/Cargo.toml` (feature `c-games-thumb = ["dep:pdsim-arm", "pdsim-arm/thumb"]` that turns on C support without Unicorn), `crates/pdsim-runner/src/lib.rs`, `app/src/main.ts`, `app/src/wasm/worker.ts`, `scripts/wasm-parity.sh`, `README.md`
- Test: `crates/pdsim-runner/src/lib.rs` (tests), `app/src/wasm/files.test.ts`, manual browser check

**Interfaces:**
- Consumes: from the Lua plan, `Instance`, `withHostPaths`, `sdkPaths`, the `loadFiles("/sdk", …)` flow and the "Load Playdate SDK folder…" button; from this plan, `Backend::Thumb` (Task 4).
- Produces:
  - Loading a C game with no SDK returns `GameError("C games need the Playdate SDK's C_API headers. Load your SDK folder (it is kept in this browser).")`. The UI shows this message next to the SDK button and highlights the button.
  - `needsSdk(entries: { path: string }[]): boolean` in `files.ts` is true when the dropped game has `pdex.elf` (at the root or under `Source/`) or is a bare `.elf`.
  - When `needsSdk` is true and no SDK is stored, the UI asks for the SDK before sending `load`.
  - `scripts/wasm-parity.sh` gains `examples/c-stress` and `calibration/bench-c` (Node build with `c-games-thumb`; native with `--cpu unicorn`), so WASM vs native Unicorn parity is checked end to end.

- [ ] **Step 1: Write the failing tests**

```rust
// pdsim-runner, cfg(feature = "c-games-thumb")
#[test] fn c_game_without_sdk_names_the_sdk()
    // load(c-stress, RunConfig { sdk: None, .. }) -> Err containing "Load your SDK folder"
```

```ts
test("needsSdk", ...)   // ["g/pdex.elf"] true; ["g/Source/pdex.elf"] true; ["game.elf"] true; ["g/main.lua"] false
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p pdsim-runner --no-default-features --features c-games-thumb && (cd app && npm test)`
Expected: FAIL.

- [ ] **Step 3: Implement** the feature, the error message, `needsSdk`, the UI prompt and the parity-script additions.

- [ ] **Step 4: Run tests and parity**

Run: `cargo test --workspace && cargo test -p pdsim-runner --no-default-features --features c-games-thumb && (cd app && npm test) && scripts/wasm-parity.sh`
Expected: PASS, `parity OK: 4 projects`.

- [ ] **Step 5: Manual acceptance (spec "Done when")**

Run `cd app && npm run build:web && npx vite preview --outDir dist`. In Chrome:
- Drop `examples/c-stress` (with `pdex.elf` built). Check that the SDK prompt appears.
- Load the SDK folder. Check that the game plays and the perf panel shows C hotspots.
- In turbo mode, check that frame 60's work time matches `pdsim run examples/c-stress --frames 61 --frames-out f.jsonl`.
- Reload and drop the game again. Check that it loads with no SDK prompt, since the SDK persists.

Note the `.wasm` gzipped size that CI prints in the commit message.

- [ ] **Step 6: Commit**

```bash
git add crates app scripts/wasm-parity.sh README.md
git commit -m "Run C games in the browser on pdsim-thumb"
```
