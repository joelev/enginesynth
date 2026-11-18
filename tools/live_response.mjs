// How fast does the live engine answer a throttle stab from idle? Prints rpm and output level over time.
import fs from 'node:fs';
const { instance } = await WebAssembly.instantiate(fs.readFileSync(new URL('../live/engsim.wasm', import.meta.url)), {});
const x = instance.exports;
for (const engine of process.argv.slice(2)) {
  const req = new TextEncoder().encode(JSON.stringify({ engine, overrides: {} }));
  const p = x.es_alloc(req.length); new Uint8Array(x.memory.buffer, p, req.length).set(req);
  const l = x.es_new(p, req.length, 0, 0); const out = x.es_alloc(128 * 8);
  const rms = () => { const v = new Float32Array(x.memory.buffer, out, 256); let s = 0; for (const a of v) s += a * a; return Math.sqrt(s / 256); };
  for (let b = 0; b < 375 * 3; b++) x.es_process(l, out, 128);
  const rpm0 = x.es_rpm(l); x.es_set_throttle(l, 1);
  let line = `${engine}: idle ${rpm0.toFixed(0)} rpm;`, t10 = null, t50 = null; const marks = [];
  for (let b = 1; b <= 300; b++) {
    x.es_process(l, out, 128); const t = b * 128 / 48; const r = x.es_rpm(l);
    if (t10 === null && r > rpm0 * 1.10) t10 = t; if (t50 === null && r > rpm0 * 1.5) t50 = t;
    if ([20, 50, 100, 150, 200, 300, 400].some(m => Math.abs(t - m) < 1.4)) marks.push(`${Math.round(t)}ms ${r.toFixed(0)}`);
  }
  console.log(line, `+10% rpm after ${t10?.toFixed(0)} ms, +50% after ${t50?.toFixed(0)} ms |`, marks.join(', '));
}
