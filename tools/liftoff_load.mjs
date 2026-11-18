// Audio-thread cost with and without lift-off pops (WebAssembly in node; V8 is Chrome's engine).
// usage: [ROUNDS=12] [ENGINES=a,b] node tools/liftoff_load.mjs [other.wasm]   (a second build, e.g. the one before this change, is timed beside it)
// Each figure is seconds of computing per second of sound over 2 s at full throttle from 70% of
// the redline (in neutral, so most of it is on the limiter) and 2 s after the lift; best of ROUNDS rounds.
import fs from 'node:fs';
const files = [[new URL('../live/engsim.wasm', import.meta.url), 'this build']];
if (process.argv[2]) files.unshift([process.argv[2], 'other build']);
const builds = [];
for (const [f, label] of files) builds.push([(await WebAssembly.instantiate(fs.readFileSync(f), {})).instance.exports, label]);
function run(x, engine, q, level) {
  const req = new TextEncoder().encode(JSON.stringify({ engine, overrides: {} }));
  const p = x.es_alloc(req.length); new Uint8Array(x.memory.buffer, p, req.length).set(req);
  const str = (pp, n) => new TextDecoder().decode(new Uint8Array(x.memory.buffer, pp, n));
  const redline = JSON.parse(str(x.es_presets_ptr(), x.es_presets_len())).find(e => e.name === engine).redline;
  const l = x.es_new(p, req.length, q, 0.7 * redline); x.es_free(p, req.length);
  if (level > 0) x.es_set_liftoff_pops(l, level);
  const out = x.es_alloc(1024); x.es_set_throttle(l, 1);
  for (let i = 0; i < 40; i++) x.es_process(l, out, 128);
  const t0 = performance.now(); for (let i = 0; i < 750; i++) x.es_process(l, out, 128); const hot = (performance.now() - t0) / 2000;
  x.es_set_throttle(l, 0);
  let worst = 0; const t1 = performance.now();
  for (let i = 0; i < 750; i++) { const a = performance.now(); x.es_process(l, out, 128); worst = Math.max(worst, performance.now() - a); }
  const lift = (performance.now() - t1) / 2000;
  x.es_drop(l); x.es_free(out, 1024);
  return [hot, lift, worst / (128 / 48)];
}
// the configurations take turns and each keeps its best, so a busy machine hurts them alike
console.log('engine          quality  build        pops  full throttle   after the lift   worst block after the lift (share of its 2.67 ms)');
const rounds = +(process.env.ROUNDS || 12);
for (const engine of (process.env.ENGINES || 'v8_cross,bmw_s70_2_v12,i4').split(',')) for (const q of [0, 1]) {
  const cfg = [];
  for (const [x, label] of builds) for (const level of (x.es_set_liftoff_pops ? [0, 1] : [0])) cfg.push({ x, label, level, best: [9, 9, 9] });
  for (let round = 0; round < rounds; round++) for (const c of cfg) { const r = run(c.x, engine, q, c.level); c.best = c.best.map((v, i) => Math.min(v, r[i])); }
  for (const c of cfg) console.log(`${engine.padEnd(15)} ${['light', 'heavy'][q].padEnd(8)} ${c.label.padEnd(12)} ${String(c.level).padEnd(5)} ${c.best[0].toFixed(3).padStart(8)} ${c.best[1].toFixed(3).padStart(15)} ${c.best[2].toFixed(2).padStart(12)}`);
}
