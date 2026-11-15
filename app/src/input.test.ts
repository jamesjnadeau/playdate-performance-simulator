import { expect, test } from "vitest";
import { angleDelta, motionToAccel, padButtons, stickAngle, type PadState } from "./input";

const pad = (pressed: number[] = [], axes: number[] = [0, 0, 0, 0]): PadState => ({
  mapping: "standard",
  buttons: Array.from({ length: 17 }, (_, i) => ({ pressed: pressed.includes(i) })),
  axes,
});
const sorted = (s: Set<string>) => [...s].sort();

test("face buttons follow Playdate's B-left, A-right layout", () => {
  expect(sorted(padButtons(pad([0])))).toEqual(["b"]);
  expect(sorted(padButtons(pad([1])))).toEqual(["a"]);
  expect(sorted(padButtons(pad([2, 3])))).toEqual(["a", "b"]);
});

test("d-pad buttons and the left stick both steer", () => {
  expect(sorted(padButtons(pad([12, 15])))).toEqual(["right", "up"]);
  expect(sorted(padButtons(pad([], [-0.9, 0.7, 0, 0])))).toEqual(["down", "left"]);
  expect(sorted(padButtons(pad([], [0.3, -0.3, 0, 0])))).toEqual([]);
});

test("a pad with fewer buttons or axes does not throw", () => {
  expect(sorted(padButtons({ buttons: [{ pressed: true }], axes: [] }))).toEqual(["b"]);
  expect(stickAngle({ buttons: [], axes: [0, 0] })).toBeNull();
});

test("stickAngle is clockwise from up, null in the dead zone", () => {
  expect(stickAngle(pad([], [0, 0, 0, -1]))).toBeCloseTo(0);
  expect(stickAngle(pad([], [0, 0, 1, 0]))).toBeCloseTo(90);
  expect(stickAngle(pad([], [0, 0, 0, 1]))).toBeCloseTo(180);
  expect(stickAngle(pad([], [0, 0, -1, 0]))).toBeCloseTo(270);
  expect(stickAngle(pad([], [0, 0, 0.3, 0.3]))).toBeNull();
});

test("angleDelta takes the short way round", () => {
  expect(angleDelta(350, 10)).toBe(20);
  expect(angleDelta(10, 350)).toBe(-20);
  expect(angleDelta(0, 180)).toBe(180);
  expect(angleDelta(90, 90)).toBe(0);
});

test("motionToAccel converts m/s² to clamped, rounded g", () => {
  expect(motionToAccel({ x: 0, y: 9.80665, z: 0 })).toEqual({ x: 0, y: 1, z: 0 });
  expect(motionToAccel({ x: -4.9, y: 30, z: -0.001 })).toEqual({ x: -0.5, y: 1, z: 0 });
  expect(motionToAccel({ x: null, y: 1, z: 1 })).toBeNull();
});
