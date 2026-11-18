// Audio-thread load of every engine at the light and heavy live levels (WebAssembly, in node).
// usage: node live_load.mjs [min-load-to-print]
import fs from 'node:fs';
const { instance } = await WebAssembly.instantiate(fs.readFileSync(new URL('../live/engsim.wasm', import.meta.url)), {});
const x = instance.exports;
const pp = x.es_presets_ptr(), pl = x.es_presets_len(); // (these may grow the memory, so take them before making a view)
const presets = JSON.parse(new TextDecoder().decode(new Uint8Array(x.memory.buffer, pp, pl)));
const floor = +(process.argv[2] || 0);
const out = x.es_alloc(1024);
for (const p of presets) {
  const row = [];
  for (const q of [0, 1]) {
    const req = new TextEncoder().encode(JSON.stringify({ engine: p.name, overrides: {} }));
    const ptr = x.es_alloc(req.length); new Uint8Array(x.memory.buffer, ptr, req.length).set(req);
    const l = x.es_new(ptr, req.length, q, 0.7 * p.redline); x.es_free(ptr, req.length);
    if (!l) { row.push('fail'); continue; }
    x.es_set_throttle(l, 1);
    for (let i = 0; i < 40; i++) x.es_process(l, out, 128);
    const t0 = performance.now(); let bad = 0;
    for (let i = 0; i < 150; i++) { x.es_process(l, out, 128); const v = new Float32Array(x.memory.buffer, out, 256); if (!Number.isFinite(v[0])) bad++; }
    row.push(((performance.now() - t0) / 400).toFixed(2) + (bad ? '!' : ''));
    x.es_drop(l);
  }
  if (+row[0] >= floor || row.includes('fail') || row.join('').includes('!')) console.log(p.name.padEnd(30), 'light', row[0], 'heavy', row[1]);
}
console.log(presets.length, 'engines');
