// Audio-thread cost of the live session (es_process, the path the browser's worklet calls), as
// seconds of computing per second of sound, median of several runs.
// usage: node tools/perf_bench.mjs [--wasm file] [--vs other.wasm ...] [--reps n] [--hash] [engine ...]
//   --vs     A/B timing of other builds against --wasm (default live/engsim.wasm), turn by turn
//   --alt a,b  the same, against quality indices a (light) and b (heavy) of the reference build
//   --hash   instead of timing, print a hash of the audio and telemetry of a scripted session
//            (idle, blip, full throttle into the limiter, lift) per engine at both qualities, and once
//            more with rich mixture, fueled overrun, spark-cut limiter and lift-off pops, to check
//            that a change leaves the live output bit-identical.
import fs from 'node:fs';
const args = process.argv.slice(2);
const opt = (k, d) => { const i = args.indexOf(k); if (i < 0) return d; const v = args[i + 1]; args.splice(i, 2); return v; };
const wasmPath = opt('--wasm', new URL('../live/engsim.wasm', import.meta.url).pathname);
const reps = +opt('--reps', 5);
const compare = []; for (let v; (v = opt('--vs', null)) !== null;) compare.push(v);
const alts = []; for (let v; (v = opt('--alt', null)) !== null;) alts.push(v.split(',').map(Number));
const hashMode = args.includes('--hash'); if (hashMode) args.splice(args.indexOf('--hash'), 1);
const engines = args.length ? args : ['single', 'i4', 'honda_f20c', 'flat6', 'v8_cross', 'bmw_s70_2_v12', 'w16', 'wankel4_pp', 'twostroke_250', 'radial7'];
const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), {});
const x = instance.exports;
const str = (p, n) => new TextDecoder().decode(new Uint8Array(x.memory.buffer, p, n));
const presets = JSON.parse(str(x.es_presets_ptr(), x.es_presets_len()));
const byName = Object.fromEntries(presets.map(p => [p.name, p]));
const out = x.es_alloc(128 * 2 * 4);
function make(engine, q, rpm) {
  const req = new TextEncoder().encode(JSON.stringify({ engine, overrides: {} }));
  const p = x.es_alloc(req.length); new Uint8Array(x.memory.buffer, p, req.length).set(req);
  const l = x.es_new(p, req.length, q, rpm); x.es_free(p, req.length);
  if (!l) throw new Error('es_new failed for ' + engine);
  return l;
}
const blocks = s => Math.round(s * 48000 / 128);
const median = a => { const b = [...a].sort((u, v) => u - v); return b[b.length >> 1]; };

if (hashMode) {
  // FNV-1a over the output bytes and the latest telemetry frame every 32 blocks
  for (const name of engines) {
    for (const q of [0, 1, 'fuel']) {
      const l = make(name, q === 'fuel' ? 0 : q, 0);
      if (q === 'fuel') { x.es_set_phi(l, 1.3); x.es_set_overrun_fuel(l, 1); x.es_set_limiter(l, 1); x.es_set_liftoff_pops(l, 0.7); }
      let h = 0x811c9dc5n;
      const mix = bytes => { for (const b of bytes) { h ^= BigInt(b); h = (h * 0x01000193n) & 0xffffffffn; } };
      const n = blocks(4);
      for (let b = 0; b < n; b++) {
        const t = b * 128 / 48000;
        x.es_set_throttle(l, t < 1 ? 0 : t < 1.3 ? 0.6 : t < 1.8 ? 0 : t < 3.2 ? 1 : 0);
        x.es_process(l, out, 128);
        mix(new Uint8Array(x.memory.buffer, out, 1024));
        if (b % 32 == 0) mix(new Uint8Array(x.memory.buffer, x.es_frame_ptr(l), x.es_frame_len(l) * 4));
      }
      console.log(`${name.padEnd(16)} ${q} ${h.toString(16).padStart(8, '0')} rpm ${x.es_rpm(l).toFixed(3)} frames ${x.es_frame_seq(l)}`);
      x.es_drop(l);
    }
  }
  process.exit(0);
}

if (compare.length || alts.length) {
  // A/B: the reference build and each other build take turns, a short run each, so a busy
  // machine slows them alike; report each one's median and the median of the per-turn ratios.
  // each contender: [module, quality index for light, for heavy]
  const mods = [[x, 0, 1]];
  for (const f of compare) mods.push([(await WebAssembly.instantiate(fs.readFileSync(f), {})).instance.exports, 0, 1]);
  for (const [a, b] of alts) mods.push([x, a, b]);
  console.log(`reference ${wasmPath}; against ${[...compare, ...alts.map(a => 'quality ' + a)].join(', ')}; ${reps} turns of 0.25 s each`);
  for (const name of engines) {
    const pr = byName[name];
    for (const q of [0, 1]) for (const wot of [false, true]) {
      const ls = mods.map(([m, ql, qh]) => {
        const req = new TextEncoder().encode(JSON.stringify({ engine: name, overrides: {} }));
        const p = m.es_alloc(req.length); new Uint8Array(m.memory.buffer, p, req.length).set(req);
        const l = m.es_new(p, req.length, q ? qh : ql, wot ? 0.7 * pr.redline : 0); m.es_free(p, req.length);
        m.es_set_throttle(l, wot ? 1 : 0);
        const o = m.es_alloc(1024);
        for (let b = 0; b < blocks(wot ? 0.3 : 2); b++) m.es_process(l, o, 128);
        return [m, l, o];
      });
      const t = mods.map(() => []);
      for (let r = 0; r < reps; r++) ls.forEach(([m, l, o], k) => {
        const n = blocks(0.25); const t0 = performance.now();
        for (let b = 0; b < n; b++) m.es_process(l, o, 128);
        t[k].push((performance.now() - t0) / 1000 / 0.25);
      });
      const cols = t.map(a => median(a).toFixed(3));
      const ratios = t.slice(1).map(a => median(a.map((v, i) => v / t[0][i])).toFixed(3));
      console.log(`${name.padEnd(16)} ${q ? 'heavy' : 'light'} ${wot ? 'wot ' : 'idle'}  ${cols.join('  ')}  ratio ${ratios.join('  ')}`);
      ls.forEach(([m, l]) => m.es_drop(l));
    }
  }
  process.exit(0);
}

console.log(`wasm ${wasmPath}, median of ${reps}`);
console.log('engine            light idle  light WOT   heavy idle  heavy WOT');
for (const name of engines) {
  const pr = byName[name];
  if (!pr) { console.log(name, 'not found'); continue; }
  const row = [];
  for (const q of [0, 1]) {
    for (const wot of [false, true]) {
      const l = make(name, q, wot ? 0.7 * pr.redline : 0);
      x.es_set_throttle(l, wot ? 1 : 0);
      for (let b = 0; b < blocks(wot ? 0.3 : 2); b++) x.es_process(l, out, 128); // settle (and warm the JIT)
      const runs = [];
      for (let r = 0; r < reps; r++) {
        const n = blocks(0.5);
        const t0 = performance.now();
        for (let b = 0; b < n; b++) x.es_process(l, out, 128);
        runs.push((performance.now() - t0) / 1000 / 0.5);
      }
      row.push(median(runs).toFixed(3) + ` (${Math.round(x.es_rpm(l))})`);
      x.es_drop(l);
    }
  }
  console.log(name.padEnd(16), row.map(s => s.padStart(17)).join(''));
}
