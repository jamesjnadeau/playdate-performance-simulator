// Pure helpers for putting a game or the Playdate SDK into the in-browser
// simulator's filesystem.
import { unzipSync } from "fflate";
import type { Command } from "../backend";

const segments = (p: string) => p.split("/").filter((s) => s !== "");

const isJunk = (segs: string[]) => segs[0] === "__MACOSX" || segs.some((s) => s.startsWith("."));

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
  const at = (name: string) => list.find((e) => e.segs.includes(name))?.segs.indexOf(name);
  const base = at("C_API") ?? at("CoreLibs");
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
