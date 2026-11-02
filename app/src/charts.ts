// Canvas charts for the perf panel.

export const CATEGORIES = ["lua", "c", "gfx", "gc", "display", "audio", "io", "other"];
const css = getComputedStyle(document.documentElement);
export const SERIES = CATEGORIES.map((_, i) => css.getPropertyValue(`--series-${i + 1}`).trim());
const SURFACE = css.getPropertyValue("--surface-1").trim();
const GRID = css.getPropertyValue("--border").trim();
const MUTED = css.getPropertyValue("--text-muted").trim();
const SECONDARY = css.getPropertyValue("--text-secondary").trim();
const CRITICAL = css.getPropertyValue("--critical").trim();

export type FrameLite = {
  index: number;
  workMs: number;
  frameMs: number;
  budgetMs: number;
  cpu: number;
  categories: Record<string, number>;
  overBudget: boolean;
  dropped: number;
  rows: number;
  memory: { lua_heap: number; images: number; c_heap: number; capacity: number };
};

export function fit(c: HTMLCanvasElement): CanvasRenderingContext2D {
  const dpr = window.devicePixelRatio || 1;
  const w = c.clientWidth;
  const h = Number(c.getAttribute("height-css") ?? c.getAttribute("height"));
  if (!c.getAttribute("height-css")) c.setAttribute("height-css", String(h));
  if (c.width !== Math.round(w * dpr) || c.height !== Math.round(h * dpr)) {
    c.width = Math.round(w * dpr);
    c.height = Math.round(h * dpr);
    c.style.height = `${h}px`;
  }
  const g = c.getContext("2d")!;
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.clearRect(0, 0, w, h);
  return g;
}

function roundTop(g: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  r = Math.min(r, w / 2, h);
  g.beginPath();
  g.moveTo(x, y + h);
  g.lineTo(x, y + r);
  g.arcTo(x, y, x + r, y, r);
  g.arcTo(x + w, y, x + w, y + r, r);
  g.lineTo(x + w, y + h);
  g.closePath();
  g.fill();
}

export type BarLayout = { x0: number; bw: number; frames: FrameLite[] };

/** Stacked frame-time bars with the budget line. Returns the layout for hit testing. */
export function drawFrames(c: HTMLCanvasElement, frames: FrameLite[], selected: number | null): BarLayout {
  const g = fit(c);
  const w = c.clientWidth;
  const h = Number(c.getAttribute("height-css"));
  const padL = 36, padB = 16, padT = 6;
  const n = Math.max(1, Math.floor((w - padL) / 5));
  const shown = frames.slice(-n);
  const budget = shown.length ? shown[shown.length - 1].budgetMs : 33.33;
  const maxMs = Math.max(budget * 1.5, ...shown.map((f) => f.workMs)) * 1.05;
  const y = (ms: number) => padT + (h - padT - padB) * (1 - ms / maxMs);
  // grid
  g.strokeStyle = GRID;
  g.fillStyle = MUTED;
  g.font = "11px system-ui";
  g.lineWidth = 1;
  const step = maxMs > 120 ? 50 : maxMs > 60 ? 20 : 10;
  for (let ms = 0; ms <= maxMs; ms += step) {
    g.beginPath();
    g.moveTo(padL, Math.round(y(ms)) + 0.5);
    g.lineTo(w, Math.round(y(ms)) + 0.5);
    g.stroke();
    g.fillText(`${ms}`, 4, y(ms) + 4);
  }
  const bw = (w - padL) / n;
  const x0 = padL + (n - shown.length) * bw;
  shown.forEach((f, i) => {
    const x = x0 + i * bw;
    let acc = 0;
    const segs = CATEGORIES.map((k, ci) => ({ ms: f.categories[k] ?? 0, color: SERIES[ci] })).filter((s) => s.ms > 0);
    segs.forEach((s, si) => {
      const top = y(acc + s.ms);
      const bot = y(acc);
      g.fillStyle = s.color;
      const hh = Math.max(0, bot - top - (si > 0 ? 1 : 0));
      if (si === segs.length - 1) roundTop(g, x + 0.5, top, Math.max(1, bw - 1.5), hh, 2);
      else g.fillRect(x + 0.5, top, Math.max(1, bw - 1.5), hh);
      acc += s.ms;
    });
    if (f.overBudget) {
      g.fillStyle = CRITICAL;
      g.fillRect(x + 0.5, h - padB + 3, Math.max(1, bw - 1.5), 3);
    }
    if (selected === f.index) {
      g.strokeStyle = SECONDARY;
      g.strokeRect(x, padT, bw, h - padT - padB);
    }
  });
  // budget line
  g.strokeStyle = CRITICAL;
  g.setLineDash([4, 3]);
  g.beginPath();
  g.moveTo(padL, Math.round(y(budget)) + 0.5);
  g.lineTo(w, Math.round(y(budget)) + 0.5);
  g.stroke();
  g.setLineDash([]);
  g.fillStyle = SECONDARY;
  g.fillText(`budget ${budget.toFixed(1)} ms`, w - 110, y(budget) - 4);
  return { x0, bw, frames: shown };
}

const MEM_KEYS: [keyof FrameLite["memory"], string][] = [
  ["lua_heap", "Lua heap"],
  ["images", "Images & fonts"],
  ["c_heap", "C heap"],
];
export const MEM_COLORS = [SERIES[0], SERIES[1], SERIES[2]];
export const MEM_LABELS = MEM_KEYS.map((k) => k[1]);

/** Stacked memory areas over the recent frames. */
export function drawMemory(c: HTMLCanvasElement, frames: FrameLite[]): BarLayout {
  const g = fit(c);
  const w = c.clientWidth;
  const h = Number(c.getAttribute("height-css"));
  const padL = 36, padT = 4, padB = 4;
  const n = Math.max(1, Math.floor((w - padL) / 5));
  const shown = frames.slice(-n);
  const used = (f: FrameLite) => f.memory.lua_heap + f.memory.images + f.memory.c_heap;
  const maxB = Math.max(1024 * 1024, ...shown.map(used)) * 1.15;
  const y = (b: number) => padT + (h - padT - padB) * (1 - b / maxB);
  g.strokeStyle = GRID;
  g.fillStyle = MUTED;
  g.font = "11px system-ui";
  g.beginPath();
  g.moveTo(padL, Math.round(y(maxB / 1.15)) + 0.5);
  g.lineTo(w, Math.round(y(maxB / 1.15)) + 0.5);
  g.stroke();
  g.fillText(`${(maxB / 1.15 / 1048576).toFixed(1)}M`, 2, y(maxB / 1.15) + 4);
  const bw = (w - padL) / n;
  const x0 = padL + (n - shown.length) * bw;
  // Draw each layer as a filled band with a 1px surface gap between layers.
  for (let k = MEM_KEYS.length - 1; k >= 0; k--) {
    g.fillStyle = MEM_COLORS[k];
    g.beginPath();
    shown.forEach((f, i) => {
      let acc = 0;
      for (let j = 0; j <= k; j++) acc += f.memory[MEM_KEYS[j][0]];
      const x = x0 + i * bw + bw / 2;
      if (i === 0) g.moveTo(x, y(acc));
      else g.lineTo(x, y(acc));
    });
    for (let i = shown.length - 1; i >= 0; i--) g.lineTo(x0 + i * bw + bw / 2, y(0));
    g.closePath();
    g.fill();
    g.strokeStyle = SURFACE;
    g.lineWidth = 2;
    g.stroke();
  }
  return { x0, bw, frames: shown };
}

/** Semicircular CPU gauge. */
export function drawGauge(c: HTMLCanvasElement, pct: number) {
  const g = fit(c);
  const w = c.clientWidth, h = 70;
  const cx = w / 2, cy = h - 8, r = 46;
  g.lineWidth = 10;
  g.lineCap = "round";
  g.strokeStyle = GRID;
  g.beginPath();
  g.arc(cx, cy, r, Math.PI, 2 * Math.PI);
  g.stroke();
  const p = Math.max(0, Math.min(100, pct)) / 100;
  g.strokeStyle = pct > 99.5 ? CRITICAL : SERIES[0];
  g.beginPath();
  g.arc(cx, cy, r, Math.PI, Math.PI + Math.PI * p);
  g.stroke();
  g.fillStyle = css.getPropertyValue("--text-primary").trim();
  g.font = "600 16px system-ui";
  g.textAlign = "center";
  g.fillText(`${Math.round(pct)}%`, cx, cy - 4);
  g.textAlign = "start";
}

export type FlameNode = { name: string; value: number; children: Map<string, FlameNode> };
export type FlameRect = { x: number; y: number; w: number; node: FlameNode; depth: number };

export function buildFlame(folded: Record<string, number>): FlameNode {
  const root: FlameNode = { name: "frame", value: 0, children: new Map() };
  for (const [stack, v] of Object.entries(folded)) {
    root.value += v;
    let cur = root;
    for (const part of stack.split(";")) {
      let next = cur.children.get(part);
      if (!next) {
        next = { name: part, value: 0, children: new Map() };
        cur.children.set(part, next);
      }
      next.value += v;
      cur = next;
    }
  }
  return root;
}

function hashColor(name: string): string {
  // Colour by kind: native API calls, GC, Lua code.
  if (name.startsWith("[gc]")) return SERIES[3];
  if (name.startsWith("[")) return SERIES[2];
  return SERIES[0];
}

/** Icicle chart (root on top). Returns rectangles for hit testing. */
export function drawFlame(c: HTMLCanvasElement, root: FlameNode | null): FlameRect[] {
  const g = fit(c);
  const w = c.clientWidth;
  const rowH = 18;
  const rects: FlameRect[] = [];
  if (!root || root.value === 0) {
    g.fillStyle = MUTED;
    g.font = "12px system-ui";
    g.fillText("Pause or step to see where a frame's time went.", 8, 20);
    return rects;
  }
  const walk = (n: FlameNode, x: number, width: number, depth: number) => {
    if (width < 1) return;
    const y = depth * rowH;
    if (y + rowH > Number(c.getAttribute("height-css"))) return;
    g.fillStyle = depth === 0 ? GRID : hashColor(n.name);
    g.fillRect(x + 0.5, y + 0.5, Math.max(0.5, width - 1), rowH - 2);
    rects.push({ x, y, w: width, node: n, depth });
    if (width > 40) {
      g.fillStyle = "#ffffff";
      g.font = "11px system-ui";
      g.save();
      g.beginPath();
      g.rect(x, y, width - 2, rowH);
      g.clip();
      g.fillText(n.name, x + 4, y + 12);
      g.restore();
    }
    let cx = x;
    const kids = [...n.children.values()].sort((a, b) => b.value - a.value);
    for (const k of kids) {
      const kw = (k.value / n.value) * width;
      walk(k, cx, kw, depth + 1);
      cx += kw;
    }
  };
  walk(root, 0, w, 0);
  return rects;
}
