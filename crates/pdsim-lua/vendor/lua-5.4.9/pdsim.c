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

/* Total instructions executed by "=[pdsim]" (API-in-Lua) chunks. */
LUA_API unsigned long long pdsim_native_instructions (lua_State *L) {
  global_State *g = G(L);
  unsigned long long n = 0;
  int i;
  for (i = 0; i < NUM_OPCODES; i++) n += g->pdsim_native_opcount[i];
  return n;
}
