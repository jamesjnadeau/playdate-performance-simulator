import { expect, test } from "vitest";
import { hostBehind } from "./pacing";

test("keeping up is not behind", () => {
  expect(hostBehind({ wallMs: 0, deviceMs: 0 }, { wallMs: 1000, deviceMs: 990 }, 1)).toBe(false);
});
test("device time advancing slowly is behind", () => {
  expect(hostBehind({ wallMs: 0, deviceMs: 0 }, { wallMs: 1000, deviceMs: 400 }, 1)).toBe(true);
});
test("speed scales the expectation", () => {
  expect(hostBehind({ wallMs: 0, deviceMs: 0 }, { wallMs: 1000, deviceMs: 500 }, 0.5)).toBe(false);
});
test("too short a window is undecided", () => {
  expect(hostBehind({ wallMs: 0, deviceMs: 0 }, { wallMs: 100, deviceMs: 0 }, 1)).toBeNull();
});
