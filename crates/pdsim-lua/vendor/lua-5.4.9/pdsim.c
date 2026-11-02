/*
** pdsim.c
** Accessors for the counters the Playdate performance simulator adds to the
** Lua VM (see the "pdsim" comments in lstate.h, lvm.c, lmem.c and lgc.c).
*/

#define LUA_CORE

#include "lprefix.h"

#include <string.h>

#include "lua.h"
#include "lopcodes.h"
#include "lstate.h"
#include "lopnames.h"

LUA_API int pdsim_num_opcodes (void) {
  return NUM_OPCODES;
}

LUA_API const char *pdsim_opname (int op) {
  if (op < 0 || op >= NUM_OPCODES) return NULL;
  return opnames[op];
}

/*
** Copies the opcode histogram into 'ops' (NUM_OPCODES entries) and the
** allocator/GC counters into 'mem' (alloced, freed, gcwork, ccalls).
*/
LUA_API void pdsim_counters (lua_State *L, unsigned long long *ops,
                             unsigned long long *mem) {
  global_State *g = G(L);
  memcpy(ops, g->pdsim_opcount, sizeof(unsigned long long) * NUM_OPCODES);
  mem[0] = g->pdsim_alloced;
  mem[1] = g->pdsim_freed;
  mem[2] = g->pdsim_gcwork;
  mem[3] = g->pdsim_ccalls;
}

/*
** Counts a C function call. Functions get a slot in a small open-addressing
** table keyed by function pointer; once it is full, calls to new functions
** are only counted in the total.
*/
void pdsim_countcfunc (lua_State *L, lua_CFunction f) {
  global_State *g = G(L);
  size_t h = (size_t)f;
  unsigned int i, probe;
  g->pdsim_ccalls++;
  h ^= h >> 7;
  for (probe = 0; probe < PDSIM_CFUNCS; probe++) {
    i = (unsigned int)(h + probe) & (PDSIM_CFUNCS - 1);
    if (g->pdsim_cfunc[i] == f) {
      g->pdsim_cfunc_calls[i]++;
      return;
    }
    if (g->pdsim_cfunc[i] == NULL) {
      g->pdsim_cfunc[i] = f;
      g->pdsim_cfunc_calls[i] = 1;
      return;
    }
  }
}

LUA_API int pdsim_num_cfunc_slots (void) {
  return PDSIM_CFUNCS;
}

/*
** Copies the per-C-function table: 'funcs' gets the function in each slot
** (NULL if unused) and 'calls' its call count; PDSIM_CFUNCS entries each.
*/
LUA_API void pdsim_cfunc_counters (lua_State *L, lua_CFunction *funcs,
                                   unsigned long long *calls) {
  global_State *g = G(L);
  memcpy(funcs, g->pdsim_cfunc, sizeof(g->pdsim_cfunc));
  memcpy(calls, g->pdsim_cfunc_calls, sizeof(g->pdsim_cfunc_calls));
}

/* Total instructions executed by "=[pdsim]" (API-in-Lua) chunks. */
LUA_API unsigned long long pdsim_native_instructions (lua_State *L) {
  global_State *g = G(L);
  unsigned long long n = 0;
  int i;
  for (i = 0; i < NUM_OPCODES; i++) n += g->pdsim_native_opcount[i];
  return n;
}
