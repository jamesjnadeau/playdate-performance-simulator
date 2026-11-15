// Pure helpers for putting a game or the Playdate SDK into the in-browser
// simulator's filesystem.
import { unzipSync } from "fflate";
import type { Command } from "../backend";

const segments = (p: string) => p.split("/").filter((s) => s !== "");

const isJunk = (segs: string[]) => segs[0] === "__MACOSX" || segs.some((s) => s.startsWith("."));

/** Whether a directory is worth descending into when listing a dropped folder. */
export const wantsDir = (name: string) => name !== "__MACOSX" && !name.startsWith(".");

/** Read only the entries that have a destination (paths decided before any bytes are read). */
export async function readKept<T extends { path: string }>(
  entries: T[],
  paths: Map<string, string>,
  read: (e: T) => Promise<ArrayBuffer>,
): Promise<{ path: string; dest: string; data: ArrayBuffer }[]> {
  const out: { path: string; dest: string; data: ArrayBuffer }[] = [];
  for (const e of entries) {
    const dest = paths.get(e.path);
    if (dest !== undefined) out.push({ path: e.path, dest, data: await read(e) });
  }
  return out;
}

/** Drop junk and return each surviving entry's path segments. */
function usable(entries: { path: string }[]): { path: string; segs: string[] }[] {
  return entries.map((e) => ({ path: e.path, segs: segments(e.path) })).filter((e) => e.segs.length > 0 && !isJunk(e.segs));
}

/** The single top-level folder shared by every entry, if there is one. */
function sharedFolder(list: { segs: string[] }[]): string | null {
  if (list.length === 0 || list.some((e) => e.segs.length < 2)) return null;
  const top = list[0].segs[0];
  return list.every((e) => e.segs[0] === top) ? top : null;
}

/** Whether the dropped game is a C game, which needs the SDK's C_API headers. */
export function needsSdk(entries: { path: string }[]): boolean {
  const list = usable(entries);
  const base = (e: { segs: string[] }) => e.segs[e.segs.length - 1];
  return list.some((e) => base(e) === "pdex.elf") || (list.length === 1 && base(list[0]).endsWith(".elf"));
}

/**
 * Why the dropped files hold nothing the simulator can run, or null if they
 * have a main.lua or a .elf. Spots the common mistake of dropping a built .pdx.
 */
export function unrunnableReason(entries: { path: string }[]): string | null {
  const names = new Set(usable(entries).map((e) => e.segs[e.segs.length - 1]));
  if (names.has("main.lua") || [...names].some((n) => n.endsWith(".elf"))) return null;
  if (names.has("pdex.bin")) {
    return "This is a built .pdx: its pdex.bin is the device binary with symbols and relocations stripped, " +
      "which the simulator cannot run. Run a device build of the game (e.g. make device) and load the " +
      "project folder with pdex.elf in it, alongside the game's assets.";
  }
  if (["pdex.dylib", "pdex.so", "pdex.dll"].some((n) => names.has(n))) {
    return "This is a Playdate Simulator build (a native pdex library), which the simulator cannot run. " +
      "Run a device build of the game (e.g. make device) and load the project folder with pdex.elf in it.";
  }
  if (names.has("main.pdz")) {
    return "This is a built .pdx: its Lua is compiled into .pdz files, which the simulator cannot read. " +
      "Load the game's source folder instead (the one with main.lua).";
  }
  return "No main.lua or pdex.elf found. Load a Lua game's source folder (with main.lua), " +
    "or a C game's project folder with its device build (pdex.elf).";
}

/** Maps each input path to its place under /game, stripping one shared top folder. */
export function gamePaths(entries: { path: string }[]): Map<string, string> {
  const list = usable(entries);
  const strip = sharedFolder(list) ? 1 : 0;
  return new Map(list.map((e) => [e.path, "/game/" + e.segs.slice(strip).join("/")]));
}

/** The stripped top-level folder name, or "game". */
export function gameName(entries: { path: string }[]): string {
  return sharedFolder(usable(entries)) ?? "game";
}

/** Like gamePaths for /sdk, keeping only CoreLibs, Resources/Fonts and C_API. */
export function sdkPaths(entries: { path: string }[]): Map<string, string> {
  const list = usable(entries);
  // The shallowest C_API or CoreLibs folder marks the SDK root.
  let base: number | undefined;
  for (const e of list) {
    for (const name of ["C_API", "CoreLibs"]) {
      const i = e.segs.indexOf(name);
      if (i >= 0 && e.segs.length - i >= 2 && (base === undefined || i < base)) base = i;
    }
  }
  const out = new Map<string, string>();
  if (base === undefined) return out;
  for (const e of list) {
    const rel = e.segs.slice(base);
    if (e.segs.length - base < 2) continue;
    const keep = rel[0] === "CoreLibs" || rel[0] === "C_API" || (rel[0] === "Resources" && rel[1] === "Fonts" && rel.length > 2);
    if (keep) out.set(e.path, "/sdk/" + rel.join("/"));
  }
  return out;
}

/** Unpack a zip into files, skipping directory entries. */
export function unzipEntries(data: Uint8Array): { path: string; data: ArrayBuffer }[] {
  const out: { path: string; data: ArrayBuffer }[] = [];
  for (const [path, bytes] of Object.entries(unzipSync(data))) {
    if (path.endsWith("/")) continue;
    out.push({ path, data: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer });
  }
  return out;
}

/** Point a `load` at the game's save folder and the SDK, if loaded. */
export function withHostPaths(cmd: Command, gameName: string, haveSdk: boolean): Command {
  if (cmd.type !== "load") return cmd;
  return { ...cmd, dataDir: `/data/${gameName}`, ...(haveSdk ? { sdk: "/sdk" } : {}) };
}

/** The download name for a trace of the frame the export was requested at. */
export function traceFileName(frame: number): string {
  return `pdsim-trace-${frame}.json`;
}
