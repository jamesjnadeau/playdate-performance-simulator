// Transport to the simulation session: Tauri IPC inside the app, or a
// WebSocket to `pdsim serve` when the page runs in a plain browser.

export type Event = { type: string; [k: string]: any };
export type Command = { type: string; [k: string]: any };

export interface Backend {
  kind: "tauri" | "websocket";
  send(cmd: Command): void;
  onEvent(cb: (e: Event) => void): void;
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
