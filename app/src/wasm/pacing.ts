// Detects the host running slower than the device being simulated. The stats
// events carry no explicit signal, so compare how far `deviceMs` advanced with
// the wall-clock time between two stats events: in device mode at speed s the
// ratio should be about 1, and well below 1 means the session keeps
// re-anchoring because the host cannot keep up.

export type PacingSample = { wallMs: number; deviceMs: number };

/** Needs this much wall time between samples to judge. */
const MIN_WINDOW_MS = 500;
/** Device time advancing less than this fraction of expected is "behind". */
const BEHIND_RATIO = 0.8;

/** Returns true/false once `cur` is far enough from `prev`, null if too soon to tell. */
export function hostBehind(prev: PacingSample, cur: PacingSample, speed: number): boolean | null {
  const wall = cur.wallMs - prev.wallMs;
  if (wall < MIN_WINDOW_MS) return null;
  const expected = wall * speed;
  return cur.deviceMs - prev.deviceMs < expected * BEHIND_RATIO;
}
