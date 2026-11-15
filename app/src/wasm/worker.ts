// Runs the emscripten build of the simulator off the main thread. The page
// sends commands and files; the worker ticks the session and posts events.
// The URL of pdsim-web.js arrives as the worker's `name`.
import type { Command } from "../backend";
import { delayFor, drainEvents } from "./events";
import { withHostPaths } from "./files";
import type { FromWorker, ToWorker } from "./protocol";

type Module = {
  _pdsim_new(): void;
  _pdsim_send(ptr: number, len: number, nowMs: number): number;
  _pdsim_tick(nowMs: number): number;
  _pdsim_poll(buf: number, cap: number): number;
  _malloc(n: number): number;
  _free(p: number): void;
  HEAPU8: Uint8Array;
  FS: {
    mkdirTree(p: string): void;
    writeFile(p: string, data: Uint8Array): void;
    readdir(p: string): string[];
    stat(p: string): { mode: number };
    isDir(mode: number): boolean;
    unlink(p: string): void;
    rmdir(p: string): void;
    mount(type: unknown, opts: object, mountpoint: string): void;
    syncfs(populate: boolean, cb: (err?: unknown) => void): void;
    analyzePath(p: string): { exists: boolean };
    filesystems: { IDBFS: unknown };
  };
};

const ctx = self as unknown as {
  name: string;
  onmessage: ((e: MessageEvent<ToWorker>) => void) | null;
  postMessage(m: FromWorker): void;
};
const post = (m: FromWorker) => ctx.postMessage(m);

const SLICE_MS = 8;
const encoder = new TextEncoder();

let m: Module;
const buf = { ptr: 0, cap: 64 * 1024 };
let timer: ReturnType<typeof setTimeout> | undefined;
let yielding: MessageChannel | undefined;
let running = false;
let syncing = false;
let gameName = "game";

// Save data (/data) and the SDK (/sdk) live in IndexedDB between visits.
const PERSISTED = ["/data", "/sdk"];
const SYNC_MS = 1000;

let syncAgain = false;
const haveSdk = () => m.FS.analyzePath("/sdk/C_API").exists;
const announceSdk = () => post({ kind: "event", event: { type: "sdk", loaded: haveSdk() } });

function syncOut() {
  if (syncing) {
    syncAgain = true;
    return;
  }
  syncing = true;
  m.FS.syncfs(false, (err) => {
    syncing = false;
    if (err) console.warn("Could not save to IndexedDB:", err);
    if (syncAgain) {
      syncAgain = false;
      syncOut();
    }
  });
}

function flush() {
  const events = drainEvents(
    (p, c) => m._pdsim_poll(p, c),
    () => m.HEAPU8,
    buf,
    (n) => m._malloc(n),
    (p) => m._free(p),
  );
  for (const event of events) {
    if (event.type === "state") {
      const was = running;
      running = !!event.running;
      if (was && !running) syncOut();
    }
    post({ kind: "event", event });
  }
}

function pump() {
  timer = undefined;
  const start = performance.now();
  let delay: number | null;
  for (;;) {
    const now = performance.now();
    delay = delayFor(m._pdsim_tick(now), now);
    if (delay !== 0 || now - start >= SLICE_MS) break;
  }
  flush();
  if (delay === null) return;
  if (delay > 0) {
    timer = setTimeout(pump, delay);
  } else {
    // Slice used up with work still due: yield to the event loop so incoming
    // commands get handled, without the 4 ms setTimeout clamp.
    yielding ??= new MessageChannel();
    yielding.port1.onmessage = () => pump();
    yielding.port2.postMessage(0);
  }
}

function schedule() {
  if (timer !== undefined) clearTimeout(timer);
  timer = undefined;
  pump();
}

function send(cmd: Command) {
  const bytes = encoder.encode(JSON.stringify(cmd));
  const p = m._malloc(bytes.length);
  m.HEAPU8.set(bytes, p);
  m._pdsim_send(p, bytes.length, performance.now());
  m._free(p);
}

// Empties a directory but never removes `dir` itself, so an IDBFS mount point
// stays mounted.
function clearDir(dir: string) {
  for (const name of m.FS.readdir(dir)) {
    if (name === "." || name === "..") continue;
    const p = `${dir}/${name}`;
    if (m.FS.isDir(m.FS.stat(p).mode)) {
      clearDir(p);
      m.FS.rmdir(p);
    } else m.FS.unlink(p);
  }
}

function writeFiles(root: string, files: { path: string; data: ArrayBuffer }[]) {
  m.FS.mkdirTree(root);
  clearDir(root);
  for (const f of files) {
    const dest = `${root}/${f.path}`;
    const slash = dest.lastIndexOf("/");
    if (slash > root.length) m.FS.mkdirTree(dest.slice(0, slash));
    m.FS.writeFile(dest, new Uint8Array(f.data));
  }
}

async function start() {
  const url = ctx.name || new URL("/pdsim-web.js", self.location.origin).href;
  const mod = await import(/* @vite-ignore */ url);
  m = (await mod.default()) as Module;
  for (const dir of PERSISTED) {
    m.FS.mkdirTree(dir);
    m.FS.mount(m.FS.filesystems.IDBFS, {}, dir);
  }
  await new Promise<void>((resolve, reject) => m.FS.syncfs(true, (err) => (err ? reject(err) : resolve())));
  announceSdk();
  setInterval(() => {
    if (running) syncOut();
  }, SYNC_MS);
  m._pdsim_new();
  buf.ptr = m._malloc(buf.cap);
  post({ kind: "ready" });
}

const ready = start().catch((err) => post({ kind: "fatal", message: `Failed to load the simulator: ${err}` }));

ctx.onmessage = async (e) => {
  await ready;
  if (!m) return;
  try {
    const msg = e.data;
    if (msg.kind === "files") {
      writeFiles(msg.root, msg.files);
      if (msg.root === "/game") gameName = msg.name ?? "game";
      else {
        announceSdk();
        syncOut();
      }
    } else {
      send(withHostPaths(msg.cmd, gameName, haveSdk()));
      schedule();
      if (msg.cmd.type === "pause" || msg.cmd.type === "step") syncOut();
    }
  } catch (err) {
    post({ kind: "event", event: { type: "error", message: String(err) } });
  }
};
