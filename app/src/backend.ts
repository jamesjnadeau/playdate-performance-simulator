// Transport to the simulation session: Tauri IPC inside the app, or a
// WebSocket to `pdsim serve`, or the simulator itself in a Web Worker (the
// default in a plain browser; `?backend=ws` or `?ws=` picks the WebSocket).

import type { FromWorker, ToWorker } from "./wasm/protocol";

export type Event = { type: string; [k: string]: any };
export type Command = { type: string; [k: string]: any };

export interface Backend {
  kind: "tauri" | "websocket" | "wasm";
  send(cmd: Command): void;
  onEvent(cb: (e: Event) => void): void;
  /**
   * Put game or SDK files into the in-browser simulator's filesystem. This
   * replaces everything under `root`, and transfers (detaches) the passed
   * ArrayBuffers. `name` is the game's folder name, used for its save data.
   */
  loadFiles?(root: "/game" | "/sdk", files: { path: string; data: ArrayBuffer }[], name?: string): void;
}

export async function connect(): Promise<Backend> {
  if ((window as any).__TAURI_INTERNALS__) {
    const { invoke } = await import("@tauri-apps/api/core");
    const { listen } = await import("@tauri-apps/api/event");
    return {
      kind: "tauri",
      send: (cmd) => void invoke("send", { cmd }),
      onEvent: (cb) => void listen<Event>("pdsim", (e) => cb(e.payload)),
    };
  }
  const params = new URLSearchParams(location.search);
  const backend = params.get("backend");
  if (backend === "wasm" || (backend !== "ws" && !params.has("ws"))) return connectWasm(params);
  const url = params.get("ws") ?? `ws://${location.hostname || "127.0.0.1"}:8765`;
  const ws = new WebSocket(url);
  const listeners: ((e: Event) => void)[] = [];
  const queue: string[] = [];
  ws.onmessage = (m) => {
    const e = JSON.parse(m.data) as Event;
    for (const l of listeners) l(e);
  };
  ws.onopen = () => {
    for (const q of queue.splice(0)) ws.send(q);
  };
  ws.onclose = () => {
    for (const l of listeners) l({ type: "error", message: `Disconnected from ${url}. Is 'pdsim serve' running?` });
  };
  return {
    kind: "websocket",
    send: (cmd) => {
      const s = JSON.stringify(cmd);
      if (ws.readyState === WebSocket.OPEN) ws.send(s);
      else queue.push(s);
    },
    onEvent: (cb) => void listeners.push(cb),
  };
}

function connectWasm(params: URLSearchParams): Backend {
  const wasmUrl = params.get("wasm") ?? new URL("/pdsim-web.js", location.origin).href;
  // The module's URL travels as the worker's name.
  const worker = new Worker(new URL("./wasm/worker.ts", import.meta.url), { type: "module", name: wasmUrl });
  const listeners: ((e: Event) => void)[] = [];
  const emit = (e: Event) => {
    for (const l of listeners) l(e);
  };
  const post = (m: ToWorker, transfer: Transferable[] = []) => worker.postMessage(m, transfer);
  // Messages sent before the module is ready wait here.
  let pending: (() => void)[] | null = [];
  const whenReady = (f: () => void) => (pending ? void pending.push(f) : f());
  worker.onmessage = (m: MessageEvent<FromWorker>) => {
    const msg = m.data;
    if (msg.kind === "event") emit(msg.event);
    else if (msg.kind === "fatal") emit({ type: "error", message: msg.message });
    else {
      const q = pending ?? [];
      pending = null;
      for (const f of q) f();
    }
  };
  worker.onerror = (e) => emit({ type: "error", message: `Simulator worker failed: ${e.message}` });
  return {
    kind: "wasm",
    send: (cmd) => whenReady(() => post({ kind: "cmd", cmd })),
    onEvent: (cb) => void listeners.push(cb),
    loadFiles: (root, files, name) =>
      whenReady(() => post({ kind: "files", root, files, name }, files.map((f) => f.data))),
  };
}
