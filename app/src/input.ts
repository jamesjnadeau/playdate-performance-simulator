// Pure helpers for the extra input sources: gamepads and device motion.

/** The parts of a Gamepad these helpers read. */
export type PadState = { mapping?: string; buttons: readonly { pressed: boolean }[]; axes: readonly number[] };

// Standard-mapping button indices. Playdate's B is on the left and A on the
// right, so the right and top face buttons are A, the bottom and left are B.
const PAD_BUTTONS: [number, string][] = [
  [0, "b"], [2, "b"], [1, "a"], [3, "a"],
  [12, "up"], [13, "down"], [14, "left"], [15, "right"],
];
export const PAD_START = 9;
const STICK = 0.5;

/** The Playdate buttons a gamepad is holding: face buttons, d-pad and the left stick. */
export function padButtons(pad: PadState): Set<string> {
  const out = new Set<string>();
  for (const [i, name] of PAD_BUTTONS) if (pad.buttons[i]?.pressed) out.add(name);
  const [x = 0, y = 0] = pad.axes;
  if (x <= -STICK) out.add("left");
  if (x >= STICK) out.add("right");
  if (y <= -STICK) out.add("up");
  if (y >= STICK) out.add("down");
  return out;
}

/**
 * The crank angle the right stick points at, in degrees clockwise from up
 * (the Playdate's convention), or null when the stick is near the centre.
 */
export function stickAngle(pad: PadState, deadzone = 0.6): number | null {
  const x = pad.axes[2] ?? 0;
  const y = pad.axes[3] ?? 0;
  if (Math.hypot(x, y) < deadzone) return null;
  return ((Math.atan2(x, -y) * 180) / Math.PI + 360) % 360;
}

/** The shortest signed turn from one angle to another, in (-180, 180]. */
export function angleDelta(from: number, to: number): number {
  let d = (((to - from) % 360) + 360) % 360;
  if (d > 180) d -= 360;
  return d;
}

const G = 9.80665;
const clamp1 = (v: number) => Math.max(-1, Math.min(1, v));
const round2 = (v: number) => Math.round(v * 100) / 100 || 0;

/**
 * A `devicemotion` reading (m/s², including gravity, in the phone's own axes)
 * as Playdate accelerometer values in g, clamped to the sliders' ±1 range.
 * Both are real accelerometers, so the axes map straight across: held
 * upright, both read about (0, 1, 0).
 */
export function motionToAccel(a: { x: number | null; y: number | null; z: number | null }): { x: number; y: number; z: number } | null {
  if (a.x == null || a.y == null || a.z == null) return null;
  return { x: round2(clamp1(a.x / G)), y: round2(clamp1(a.y / G)), z: round2(clamp1(a.z / G)) };
}
