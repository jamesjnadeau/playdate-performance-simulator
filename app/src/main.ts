import { connect, type Event } from "./backend";
import {
  CATEGORIES, SERIES, MEM_COLORS, MEM_LABELS, buildFlame, drawFlame, drawFrames, drawGauge, drawMemory,
  type BarLayout, type FlameRect, type FrameLite,
} from "./charts";

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

const screen = $<HTMLCanvasElement>("screen");
const overlay = $<HTMLCanvasElement>("overlay");
const sctx = screen.getContext("2d")!;
const octx = overlay.getContext("2d")!;
const image = sctx.createImageData(400, 240);
const css = getComputedStyle(document.documentElement);
const hex = (v: string) => {
  const s = css.getPropertyValue(v).trim().replace("#", "");
  return [parseInt(s.slice(0, 2), 16), parseInt(s.slice(2, 4), 16), parseInt(s.slice(4, 6), 16)];
};
const LIGHT = hex("--lcd-light");
const DARK = hex("--lcd-dark");

const frames: FrameLite[] = [];
const MAX_FRAMES = 600;
let running = false;
let selected: number | null = null;
let barLayout: BarLayout = { x0: 0, bw: 1, frames: [] };
let memLayout: BarLayout = { x0: 0, bw: 1, frames: [] };
let flameRects: FlameRect[] = [];
let flameRoot: ReturnType<typeof buildFlame> | null = null;
let flameTotal = 0;
let cpuHz = 168e6;
let showRows = false;
let pendingFrame: Event | null = null;

const backend = await connect();

// ---- legends
$("legend").innerHTML = CATEGORIES.map((c, i) => `<span><i style="background:${SERIES[i]}"></i>${c}</span>`).join("") +
  `<span><i style="background:${css.getPropertyValue("--critical")}"></i>over budget</span>`;
$("memLegend").innerHTML = MEM_LABELS.map((l, i) => `<span><i style="background:${MEM_COLORS[i]}"></i>${l}</span>`).join("");

// ---- screen
function b64bytes(s: string): Uint8Array {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

function drawScreen(e: Event) {
  const fb = b64bytes(e.fb);
  const inv = !!e.inverted;
  const d = image.data;
  for (let y = 0; y < 240; y++) {
    for (let x = 0; x < 400; x++) {
      let white = (fb[y * 52 + (x >> 3)] >> (7 - (x & 7))) & 1;
      if (inv) white ^= 1;
      const c = white ? LIGHT : DARK;
      const o = (y * 400 + x) * 4;
      d[o] = c[0]; d[o + 1] = c[1]; d[o + 2] = c[2]; d[o + 3] = 255;
    }
  }
  sctx.putImageData(image, 0, 0);
  octx.clearRect(0, 0, 400, 240);
  if (showRows) {
    const rows = b64bytes(e.rows);
    octx.fillStyle = "rgba(230, 103, 103, 0.35)";
    for (let y = 0; y < 240; y++) if ((rows[y >> 3] >> (y & 7)) & 1) octx.fillRect(0, y, 400, 1);
  }
}

function setScale(s: number) {
  for (const c of [screen, overlay]) {
    c.style.width = `${400 * s}px`;
    c.style.height = `${240 * s}px`;
  }
}
$<HTMLSelectElement>("scale").onchange = (e) => setScale(Number((e.target as HTMLSelectElement).value));
$<HTMLInputElement>("rows").onchange = (e) => (showRows = (e.target as HTMLInputElement).checked);
setScale(2);

// ---- toolbar
const pathInput = $<HTMLInputElement>("path");
pathInput.value = localStorageGet("pdsim.path") ?? "";
const calibInput = $<HTMLInputElement>("calibPath");
calibInput.value = localStorageGet("pdsim.calibration") ?? "";
function open() {
  const path = pathInput.value.trim();
  if (!path) return;
  localStorageSet("pdsim.path", path);
  localStorageSet("pdsim.calibration", calibInput.value.trim());
  frames.length = 0;
  selected = null;
  flameRoot = null;
  $("log").textContent = "";
  backend.send({ type: "load", path, calibration: calibInput.value.trim() || null });
}
$("open").onclick = open;
pathInput.onkeydown = (e) => e.key === "Enter" && open();
calibInput.onkeydown = (e) => e.key === "Enter" && open();
const playBtn = $<HTMLButtonElement>("play");
const togglePlay = () => backend.send({ type: running ? "pause" : "play" });
playBtn.onclick = togglePlay;
$("step").onclick = () => backend.send({ type: "step" });
$<HTMLSelectElement>("mode").onchange = (e) => backend.send({ type: "setMode", mode: (e.target as HTMLSelectElement).value });
$<HTMLInputElement>("break").onchange = (e) => backend.send({ type: "breakOnOverBudget", on: (e.target as HTMLInputElement).checked });
$("trace").onclick = () => backend.send({ type: "exportTrace" });

function localStorageGet(k: string): string | null {
  try { return localStorage.getItem(k); } catch { return null; }
}
function localStorageSet(k: string, v: string) {
  try { localStorage.setItem(k, v); } catch { /* ignore */ }
}

function toast(msg: string, error = false) {
  const t = $("toast");
  t.textContent = msg;
  t.className = error ? "toast error" : "toast";
  t.style.display = "block";
  clearTimeout((toast as any).timer);
  (toast as any).timer = setTimeout(() => (t.style.display = "none"), error ? 12000 : 4000);
}

// ---- input
const keyMap: Record<string, string> = {
  ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right",
  x: "a", X: "a", k: "a", K: "a", z: "b", Z: "b", j: "b", J: "b",
};
const held = new Set<string>();
function button(name: string, down: boolean) {
  if (down === held.has(name)) return;
  if (down) held.add(name); else held.delete(name);
  document.querySelector(`[data-btn="${name}"]`)?.classList.toggle("held", down);
  backend.send({ type: "button", name, down });
}
window.addEventListener("keydown", (e) => {
  if (e.target instanceof HTMLInputElement && e.target.type !== "checkbox" && e.target.type !== "range") return;
  if (keyMap[e.key]) { button(keyMap[e.key], true); e.preventDefault(); }
  else if (e.key === " ") { togglePlay(); e.preventDefault(); }
  else if (e.key === ".") backend.send({ type: "step" });
});
window.addEventListener("keyup", (e) => keyMap[e.key] && button(keyMap[e.key], false));
window.addEventListener("blur", () => [...held].forEach((b) => button(b, false)));
document.querySelectorAll<HTMLButtonElement>("[data-btn]").forEach((b) => {
  const name = b.dataset.btn!;
  b.onpointerdown = () => button(name, true);
  b.onpointerup = b.onpointerleave = () => button(name, false);
});

// crank
const crank = $<HTMLCanvasElement>("crank");
const cctx = crank.getContext("2d")!;
let crankAngle = 0;
const docked = $<HTMLInputElement>("docked");
function drawCrank() {
  cctx.clearRect(0, 0, 120, 120);
  cctx.strokeStyle = css.getPropertyValue("--border");
  cctx.lineWidth = 8;
  cctx.beginPath(); cctx.arc(60, 60, 44, 0, Math.PI * 2); cctx.stroke();
  const a = (crankAngle - 90) * Math.PI / 180;
  cctx.strokeStyle = docked.checked ? css.getPropertyValue("--text-muted") : css.getPropertyValue("--accent");
  cctx.lineWidth = 6; cctx.lineCap = "round";
  cctx.beginPath(); cctx.moveTo(60, 60); cctx.lineTo(60 + Math.cos(a) * 44, 60 + Math.sin(a) * 44); cctx.stroke();
  cctx.fillStyle = cctx.strokeStyle;
  cctx.beginPath(); cctx.arc(60 + Math.cos(a) * 44, 60 + Math.sin(a) * 44, 7, 0, Math.PI * 2); cctx.fill();
  $("crankAngle").textContent = `${Math.round(crankAngle)}°`;
}
function turn(delta: number) {
  if (docked.checked) { docked.checked = false; backend.send({ type: "dock", docked: false }); }
  crankAngle = ((crankAngle + delta) % 360 + 360) % 360;
  backend.send({ type: "crank", delta });
  drawCrank();
}
crank.addEventListener("wheel", (e) => { e.preventDefault(); turn(e.deltaY / 4); }, { passive: false });
let dragging = false;
let lastDrag = 0;
const angleAt = (e: PointerEvent) => {
  const r = crank.getBoundingClientRect();
  return (Math.atan2(e.clientY - r.top - 60, e.clientX - r.left - 60) * 180 / Math.PI + 90 + 360) % 360;
};
crank.onpointerdown = (e) => { dragging = true; lastDrag = angleAt(e); crank.setPointerCapture(e.pointerId); };
crank.onpointermove = (e) => {
  if (!dragging) return;
  const a = angleAt(e);
  let d = a - lastDrag;
  if (d > 180) d -= 360; else if (d < -180) d += 360;
  lastDrag = a;
  turn(d);
};
crank.onpointerup = () => (dragging = false);
docked.onchange = () => { backend.send({ type: "dock", docked: docked.checked }); drawCrank(); };
drawCrank();
for (const id of ["ax", "ay", "az"]) {
  $<HTMLInputElement>(id).oninput = () => backend.send({
    type: "accel",
    x: Number($<HTMLInputElement>("ax").value), y: Number($<HTMLInputElement>("ay").value), z: Number($<HTMLInputElement>("az").value),
  });
}

// ---- charts
const framesCanvas = $<HTMLCanvasElement>("frames");
const memCanvas = $<HTMLCanvasElement>("memory");
const flameCanvas = $<HTMLCanvasElement>("flame");

function redrawCharts() {
  barLayout = drawFrames(framesCanvas, frames, selected);
  memLayout = drawMemory(memCanvas, frames);
  flameRects = drawFlame(flameCanvas, flameRoot);
}
window.addEventListener("resize", redrawCharts);

const fmtMs = (ms: number) => `${ms.toFixed(2)} ms`;
const fmtKB = (b: number) => (b >= 1048576 ? `${(b / 1048576).toFixed(2)} MB` : `${(b / 1024).toFixed(1)} KB`);

function showTip(tip: HTMLElement, canvas: HTMLCanvasElement, x: number, html: string) {
  tip.innerHTML = html;
  tip.style.display = "block";
  const max = canvas.clientWidth - tip.offsetWidth - 4;
  tip.style.left = `${Math.max(0, Math.min(max, x + 12))}px`;
  tip.style.top = "8px";
}
function frameAt(l: BarLayout, canvas: HTMLCanvasElement, e: MouseEvent) {
  const x = e.clientX - canvas.getBoundingClientRect().left;
  const i = Math.floor((x - l.x0) / l.bw);
  return { f: l.frames[i], x };
}
framesCanvas.onmousemove = (e) => {
  const { f, x } = frameAt(barLayout, framesCanvas, e);
  const tip = $("tip");
  if (!f) { tip.style.display = "none"; return; }
  const rows = CATEGORIES.filter((c) => (f.categories[c] ?? 0) > 0)
    .map((c) => `<div class="row"><span><i style="display:inline-block;width:8px;height:8px;border-radius:2px;background:${SERIES[CATEGORIES.indexOf(c)]}"></i> ${c}</span><b>${fmtMs(f.categories[c])}</b></div>`).join("");
  showTip(tip, framesCanvas, x,
    `<div><b>Frame ${f.index}</b>${f.overBudget ? ` · <span style="color:var(--critical)">over budget, ${f.dropped} dropped</span>` : ""}</div>` +
    `<div class="row"><span>work</span><b>${fmtMs(f.workMs)}</b></div>` +
    `<div class="row"><span>CPU</span><b>${f.cpu.toFixed(0)}%</b></div>` +
    `<div class="row"><span>rows sent</span><b>${f.rows}</b></div>` + rows);
};
framesCanvas.onmouseleave = () => ($("tip").style.display = "none");
framesCanvas.onclick = (e) => {
  const { f } = frameAt(barLayout, framesCanvas, e);
  if (!f) return;
  selected = f.index;
  backend.send({ type: "flame", index: f.index });
  redrawCharts();
};
memCanvas.onmousemove = (e) => {
  const { f, x } = frameAt(memLayout, memCanvas, e);
  const tip = $("memTip");
  if (!f) { tip.style.display = "none"; return; }
  const m = f.memory;
  showTip(tip, memCanvas, x, `<div><b>Frame ${f.index}</b></div>` +
    [["Lua heap", m.lua_heap], ["Images & fonts", m.images], ["C heap", m.c_heap]]
      .map(([l, v]) => `<div class="row"><span>${l}</span><b>${fmtKB(v as number)}</b></div>`).join(""));
};
memCanvas.onmouseleave = () => ($("memTip").style.display = "none");
flameCanvas.onmousemove = (e) => {
  const r = flameCanvas.getBoundingClientRect();
  const x = e.clientX - r.left, y = e.clientY - r.top;
  const hit = flameRects.find((q) => x >= q.x && x < q.x + q.w && y >= q.y && y < q.y + 18);
  const tip = $("flameTip");
  if (!hit) { tip.style.display = "none"; return; }
  const ms = (hit.node.value / cpuHz) * 1000;
  showTip(tip, flameCanvas, x, `<div><b>${escapeHtml(hit.node.name)}</b></div>` +
    `<div class="row"><span>time</span><b>${fmtMs(ms)}</b></div>` +
    `<div class="row"><span>of frame</span><b>${((hit.node.value / Math.max(1, flameTotal)) * 100).toFixed(1)}%</b></div>`);
};
flameCanvas.onmouseleave = () => ($("flameTip").style.display = "none");

function escapeHtml(s: string) {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
}

function updateTiles(e: Event) {
  const recent = frames.slice(-30);
  const avgWork = recent.reduce((s, f) => s + f.workMs, 0) / Math.max(1, recent.length);
  const avgFrame = recent.reduce((s, f) => s + f.frameMs, 0) / Math.max(1, recent.length);
  const cpu = recent.reduce((s, f) => s + f.cpu, 0) / Math.max(1, recent.length);
  $("tFrame").textContent = String(e.frameIndex);
  $("tFps").textContent = avgFrame > 0 ? (1000 / avgFrame).toFixed(1) : "–";
  $("tWork").textContent = fmtMs(avgWork);
  $("tOver").textContent = String(e.overBudgetTotal);
  $("tOver").classList.toggle("bad", e.overBudgetTotal > 0);
  $("tDrop").textContent = String(e.droppedTotal);
  $("tDrop").classList.toggle("bad", e.droppedTotal > 0);
  drawGauge($<HTMLCanvasElement>("gauge"), cpu);
  const m = e.memory;
  if (m) $("memLabel").textContent = `${fmtKB(m.lua_heap + m.images + m.c_heap)} of ${fmtKB(m.capacity)}`;
  const tbody = $("hot").querySelector("tbody")!;
  const total = e.hotspots.reduce((s: number, h: any) => s + h.self_cycles, 0) || 1;
  tbody.innerHTML = e.hotspots.slice(0, 40).map((h: any) =>
    `<tr><td>${escapeHtml(h.name)}</td><td>${((h.self_cycles / total) * 100).toFixed(1)}%</td><td>${((h.total_cycles / total) * 100).toFixed(1)}%</td></tr>`).join("");
}

// Draw at most once per animation frame.
function schedule() {
  if ((schedule as any).queued) return;
  (schedule as any).queued = true;
  requestAnimationFrame(() => {
    (schedule as any).queued = false;
    if (pendingFrame) { drawScreen(pendingFrame); pendingFrame = null; }
  });
}

backend.onEvent((e) => {
  switch (e.type) {
    case "frame":
      pendingFrame = e;
      schedule();
      break;
    case "stats":
      frames.push(...e.frames);
      if (frames.length > MAX_FRAMES) frames.splice(0, frames.length - MAX_FRAMES);
      updateTiles(e);
      redrawCharts();
      break;
    case "flame":
      flameRoot = buildFlame(e.folded);
      flameTotal = e.workCycles;
      selected = e.index;
      $("flameLabel").textContent = `frame ${e.index}`;
      redrawCharts();
      break;
    case "state":
      running = e.running;
      playBtn.textContent = running ? "Pause" : "Play";
      $<HTMLSelectElement>("mode").value = e.mode;
      $<HTMLInputElement>("break").checked = e.breakOnOverBudget;
      break;
    case "loaded":
      cpuHz = e.cpuHz;
      pathInput.value = e.path;
      $("calib").textContent = e.calibration;
      $("calib").classList.toggle("est", e.estimated);
      toast(e.description);
      break;
    case "log": {
      const log = $("log");
      log.textContent += e.lines.join("\n") + "\n";
      if (log.textContent!.length > 20000) log.textContent = log.textContent!.slice(-15000);
      log.scrollTop = log.scrollHeight;
      break;
    }
    case "break":
      toast(`Paused: frame went ${e.reason}`);
      break;
    case "traceExported":
      toast(`Trace of ${e.frames} frames written to ${e.path}\nOpen it in ui.perfetto.dev or chrome://tracing.`);
      break;
    case "error":
      toast(e.message, true);
      break;
  }
});

redrawCharts();
const initial = new URLSearchParams(location.search).get("game");
if (initial) { pathInput.value = initial; open(); }
backend.send({ type: "hello" });
