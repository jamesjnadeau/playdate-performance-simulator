import { expect, test } from "vitest";
import { gameName, gamePaths, sdkPaths, unzipEntries, withHostPaths } from "./files";
import { zipSync } from "fflate";

const e = (...paths: string[]) => paths.map((path) => ({ path }));
const obj = (m: Map<string, string>) => Object.fromEntries(m);

test("gamePaths strips one shared top folder", () => {
  expect(obj(gamePaths(e("mygame/main.lua", "mygame/img/a.png")))).toEqual({
    "mygame/main.lua": "/game/main.lua",
    "mygame/img/a.png": "/game/img/a.png",
  });
});

test("gamePaths keeps Source/ layout", () => {
  expect(obj(gamePaths(e("mygame/Source/main.lua")))).toEqual({ "mygame/Source/main.lua": "/game/Source/main.lua" });
});

test("gamePaths with no shared folder", () => {
  expect(obj(gamePaths(e("main.lua", "b.lua")))).toEqual({ "main.lua": "/game/main.lua", "b.lua": "/game/b.lua" });
  expect(obj(gamePaths(e("a/main.lua", "b/x.lua")))).toEqual({ "a/main.lua": "/game/a/main.lua", "b/x.lua": "/game/b/x.lua" });
});

test("gamePaths drops zip and OS junk", () => {
  const m = gamePaths(e("__MACOSX/x", "mygame/.DS_Store", "mygame/main.lua", "mygame/img/.hidden"));
  expect([...m.keys()]).toEqual(["mygame/main.lua"]);
  expect(m.get("mygame/main.lua")).toBe("/game/main.lua");
});

test("gameName", () => {
  expect(gameName(e("mygame/main.lua", "mygame/a/b.lua"))).toBe("mygame");
  expect(gameName(e("__MACOSX/x", "mygame/main.lua"))).toBe("mygame");
  expect(gameName(e("main.lua", "b.lua"))).toBe("game");
});

test("sdkPaths keeps only the three SDK subtrees", () => {
  const m = sdkPaths(
    e(
      "PlaydateSDK/CoreLibs/graphics.lua",
      "PlaydateSDK/Resources/Fonts/a.fnt",
      "PlaydateSDK/Resources/Other/b.png",
      "PlaydateSDK/C_API/pd_api.h",
      "PlaydateSDK/bin/pdc",
      "PlaydateSDK/.DS_Store",
    ),
  );
  expect(obj(m)).toEqual({
    "PlaydateSDK/CoreLibs/graphics.lua": "/sdk/CoreLibs/graphics.lua",
    "PlaydateSDK/Resources/Fonts/a.fnt": "/sdk/Resources/Fonts/a.fnt",
    "PlaydateSDK/C_API/pd_api.h": "/sdk/C_API/pd_api.h",
  });
});

test("sdkPaths works with no wrapping folder", () => {
  expect([...sdkPaths(e("C_API/pd_api.h", "CoreLibs/x.lua")).values()].sort()).toEqual(["/sdk/C_API/pd_api.h", "/sdk/CoreLibs/x.lua"]);
});

test("load injects dataDir under /data", () => {
  const cmd = { type: "load", path: "/game" };
  expect(withHostPaths(cmd, "mygame", true)).toEqual({ ...cmd, dataDir: "/data/mygame", sdk: "/sdk" });
  expect(withHostPaths(cmd, "mygame", false)).toEqual({ ...cmd, dataDir: "/data/mygame" });
  const other = { type: "play" };
  expect(withHostPaths(other, "mygame", true)).toBe(other);
});

test("unzipEntries skips directories", () => {
  const zip = zipSync({ "g/main.lua": new Uint8Array([1, 2, 3]), "g/img/": new Uint8Array() });
  const out = unzipEntries(zip);
  expect(out.map((x) => x.path)).toEqual(["g/main.lua"]);
  expect(new Uint8Array(out[0].data)).toEqual(new Uint8Array([1, 2, 3]));
});
