# Vendored Lua 5.4.9

`lua-5.4.9/` is Lua 5.4.9 (MIT license, Copyright © 1994–2025 Lua.org,
PUC-Rio; the full notice is at the end of `lua.h`), taken from the `lua-src`
crate, with a small patch for the simulator. `pdsim-lua-5.4.9.patch` is the
diff against the pristine sources; the new file `pdsim.c` is not in the diff.

What the patch adds, all to `global_State`:

- `pdsim_opcount[op]` — incremented for every executed instruction
  (`vmfetch` in `lvm.c`). The simulator multiplies the histogram by the
  calibrated cost of each opcode.
- `pdsim_alloced` / `pdsim_freed` — bytes through the allocator (`lmem.c`).
- `pdsim_gcwork` — units of incremental collector work (`singlestep` in
  `lgc.c`).
- `pdsim_ccalls` — C functions called by game code (`precallC` in `ldo.c`),
  charged at an average calibrated cost.
- `pdsim_cfunc[]` / `pdsim_cfunc_calls[]` — the same calls per C function, in
  a small open-addressed table keyed by function pointer (`pdsim_countcfunc`
  in `pdsim.c`). The simulator names the standard library's functions
  (`math.sin`, `next`, ...) and adds each one's calibrated extra cost.
- `pdsim_native_opcount[op]` — instructions run by functions from chunks named
  `=[pdsim]...` (the simulator's API written in Lua). These, and the C
  functions they call, are not charged as game code; the API charges its own
  calibrated cost.

`pdsim.c` exports `pdsim_counters`, `pdsim_cfunc_counters`,
`pdsim_native_instructions`, `pdsim_num_cfunc_slots`, `pdsim_num_opcodes` and
`pdsim_opname`.

`build.rs` also defines `luai_makeseed` to a constant so string hashing (and
so table iteration order) is the same in every run.
