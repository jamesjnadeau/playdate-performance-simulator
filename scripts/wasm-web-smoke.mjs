// Smoke test for the browser build under Node: loads examples/lua-stress into
// the module's in-memory FS, runs 60 turbo frames and checks the events.
// Build first: cargo build --release -p pdsim-wasm --target wasm32-unknown-emscripten --bin pdsim-web
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const built = path.join(root, 'target/wasm32-unknown-emscripten/release/pdsim-web.js');
const { default: createPdsim } = await import(pathToFileURL(built).href);
const m = await createPdsim();

function copyTree(src, dst) {
  m.FS.mkdirTree(dst);
  for (const e of fs.readdirSync(src, { withFileTypes: true })) {
    const s = path.join(src, e.name);
    const d = `${dst}/${e.name}`;
    if (e.isDirectory()) copyTree(s, d);
    else m.FS.writeFile(d, fs.readFileSync(s));
  }
}
copyTree(path.join(root, 'examples/lua-stress'), '/game');

const enc = new TextEncoder();
const dec = new TextDecoder();
function send(obj, now) {
  const bytes = enc.encode(JSON.stringify(obj));
  const p = m._malloc(bytes.length);
  m.HEAPU8.set(bytes, p);
  const rc = m._pdsim_send(p, bytes.length, now);
  m._free(p);
  return rc;
}
function drain(events) {
  const cap = 1 << 20;
  const buf = m._malloc(cap);
  for (;;) {
    const n = m._pdsim_poll(buf, cap);
    if (n === 0) break;
    if (n > cap) throw new Error(`event of ${n} bytes exceeds buffer`);
    events.push(JSON.parse(dec.decode(m.HEAPU8.slice(buf, buf + n))));
  }
  m._free(buf);
}

m._pdsim_new();
const events = [];
let now = 0;
if (send({ type: 'load', path: '/game', dataDir: '/data/lua-stress' }, now) !== 0) throw new Error('load rejected');
send({ type: 'setMode', mode: 'turbo' }, now);
for (let i = 0; i < 60; i++) {
  now += 1;
  m._pdsim_tick(now);
  drain(events);
}
drain(events);

const count = (t) => events.filter((e) => e.type === t).length;
const frames = count('frame');
if (count('loaded') !== 1 || frames !== 60 || count('error') !== 0) {
  console.error('smoke FAILED', { loaded: count('loaded'), frames, errors: events.filter((e) => e.type === 'error') });
  process.exit(1);
}
console.log(`smoke OK: ${frames} frames`);
