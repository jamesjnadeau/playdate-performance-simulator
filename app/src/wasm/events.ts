// Pure helpers for the worker's poll/tick loop. No worker or DOM globals, so
// they run under vitest in node.
import type { Event } from "../backend";

const decoder = new TextDecoder();

/** Pop every queued event out of the module. `poll` returns 0 when empty,
 *  otherwise the next event's length, copying and popping it only if it fits
 *  in `cap`. When it doesn't fit, `buf` is reallocated and the poll retried. */
export function drainEvents(
  poll: (ptr: number, cap: number) => number,
  heap: () => Uint8Array,
  buf: { ptr: number; cap: number },
  alloc: (n: number) => number,
  free: (p: number) => void,
): Event[] {
  const out: Event[] = [];
  for (;;) {
    const n = poll(buf.ptr, buf.cap);
    if (n === 0) return out;
    if (n > buf.cap) {
      let cap = buf.cap;
      while (cap < n) cap *= 2;
      free(buf.ptr);
      buf.ptr = alloc(cap);
      buf.cap = cap;
      continue;
    }
    out.push(JSON.parse(decoder.decode(heap().slice(buf.ptr, buf.ptr + n))) as Event);
  }
}

/** Milliseconds until the next tick is due; null when the session is idle. */
export function delayFor(nextWakeMs: number, nowMs: number): number | null {
  return nextWakeMs === -1 ? null : Math.max(0, nextWakeMs - nowMs);
}
