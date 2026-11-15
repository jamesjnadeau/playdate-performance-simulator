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

The patch also counts memory at the object sizes of the 32-bit device
(ARM32, the same as wasm32), on every target, so the GC paces, and
`collectgarbage("count")` and the reported Lua heap read, the same natively
as in the browser and close to the device:

- Every `luaM_` allocation macro (`lmem.h`) carries a device size beside the
  real one. Fixed-size objects use `PDSIM_DSIZE(t)`, a C11 `_Generic` table in
  `lstate.h` (an unlisted type is a compile error); variable-size objects use
  `pdsim_dsize*` helpers mirroring `lfunc.h` and `lobject.h`. `GCdebt`,
  `totalbytes` (from `PDSIM_DSIZE(LG)`), `WORK2MEM`, `pdsim_alloced` and
  `pdsim_freed` use device sizes; the real allocator still gets real sizes.
  Each free passes the same device size as its allocation.
- On 32-bit targets `lstate.c` static-asserts every table entry and header
  offset against `sizeof`/`offsetof`.
- A userdata's payload size is chosen by its creator, so each userdata block
  carries a trailer recording the payload's device size.
  `pdsim_newuserdatauv` (in `lapi.c`, declared in `lua.h`) takes it; the
  standard library passes its own (`GMatchState`, `UBox`, `LStream`,
  `RanState`, each static-asserted on 32-bit). Plain `lua_newuserdatauv`,
  used by the host (mlua's callbacks and API objects), counts no payload:
  the header and user values only.
- `LUAL_BUFFERSIZE` (`luaconf.h`) is fixed at the device value, 512, so
  string buffers move to the heap at the same lengths everywhere.

`pdsim.c` exports `pdsim_counters`, `pdsim_cfunc_counters`,
`pdsim_native_instructions`, `pdsim_num_cfunc_slots`, `pdsim_num_opcodes` and
`pdsim_opname`.

`build.rs` also defines `luai_makeseed` to a constant so string hashing (and
so table iteration order) is the same in every run.
