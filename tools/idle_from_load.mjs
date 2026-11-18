// Idle speed of every engine from load, with no throttle blip, in the app's own WebAssembly build.
// usage: node tools/idle_from_load.mjs [engine ...]   (the last column is late idle speed over target)
import fs from 'node:fs';
const { instance } = await WebAssembly.instantiate(fs.readFileSync(new URL('../live/engsim.wasm', import.meta.url)), {});
const x = instance.exports;
const str = (p, n) => new TextDecoder().decode(new Uint8Array(x.memory.buffer, p, n));
const presets = JSON.parse(str(x.es_presets_ptr(), x.es_presets_len()));
const out = x.es_alloc(128 * 2 * 4);
const only = process.argv.slice(2);
for (const pr of presets) {
  if (only.length && !only.includes(pr.name)) continue;
  for (const q of [0, 1]) {
    const req = new TextEncoder().encode(JSON.stringify({ engine: pr.name, overrides: {} }));
    const p = x.es_alloc(req.length); new Uint8Array(x.memory.buffer, p, req.length).set(req);
    const l = x.es_new(p, req.length, q, 0); x.es_free(p, req.length);
    const h = JSON.parse(str(x.es_header_ptr(l), x.es_header_len(l)));
    const r = [];
    for (let s = 0; s < 20; s++) { for (let b = 0; b < 375; b++) x.es_process(l, out, 128); r.push(Math.round(x.es_rpm(l))); }
    const tgt = h.idle_rpm ?? h.idle ?? pr.idle ?? 0;
    const late = r.slice(15).reduce((a, b) => a + b, 0) / 5;
    console.log(`${pr.name.padEnd(28)} ${h.quality.padEnd(9)} target ${tgt}  at 1s ${r[0]} 3s ${r[2]} 5s ${r[4]} 10s ${r[9]} 20s ${r[19]}  late/target ${tgt ? (late / tgt).toFixed(2) : '?'}`);
    x.es_drop(l);
  }
}
