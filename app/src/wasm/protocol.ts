// Messages between the page and the wasm worker.
import type { Command, Event } from "../backend";

export type FileRoot = "/game" | "/sdk";
export type WasmFile = { path: string; data: ArrayBuffer };

export type ToWorker =
  | { kind: "cmd"; cmd: Command }
  | { kind: "files"; root: FileRoot; files: WasmFile[] };

export type FromWorker =
  | { kind: "ready" }
  | { kind: "event"; event: Event }
  | { kind: "fatal"; message: string };
