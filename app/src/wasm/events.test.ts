import { expect, test } from "vitest";
import { delayFor, drainEvents } from "./events";

// A fake module heap with a bump allocator and a poll backed by a queue.
function fake(events: object[]) {
  const heap = new Uint8Array(1 << 20);
  let next = 8;
  const freed: number[] = [];
  const queue = events.map((e) => new TextEncoder().encode(JSON.stringify(e)));
  const poll = (ptr: number, cap: number) => {
    if (queue.length === 0) return 0;
    const bytes = queue[0];
    if (bytes.length > cap) return bytes.length;
    heap.set(bytes, ptr);
    queue.shift();
    return bytes.length;
  };
  const alloc = (n: number) => {
    const p = next;
    next += n;
    return p;
  };
  return { heap, poll, alloc, free: (p: number) => void freed.push(p), freed };
}

test("drainEvents returns all queued events in order", () => {
  const evs = [{ type: "a", n: 1 }, { type: "b", n: 2 }, { type: "c", n: 3 }];
  const f = fake(evs);
  const buf = { ptr: f.alloc(65536), cap: 65536 };
  const got = drainEvents(f.poll, () => f.heap, buf, f.alloc, f.free);
  expect(got).toEqual(evs);
});

test("drainEvents grows the buffer for a large event", () => {
  const big = { type: "traceData", data: "x".repeat(200 * 1024) };
  const f = fake([big]);
  const buf = { ptr: f.alloc(65536), cap: 65536 };
  const oldPtr = buf.ptr;
  const got = drainEvents(f.poll, () => f.heap, buf, f.alloc, f.free);
  expect(got).toEqual([big]);
  expect(buf.cap).toBe(262144);
  expect(f.freed).toContain(oldPtr);
});

test("delayFor is null when idle and never negative", () => {
  expect(delayFor(-1, 5)).toBeNull();
  expect(delayFor(3, 5)).toBe(0);
  expect(delayFor(40, 5)).toBe(35);
});
