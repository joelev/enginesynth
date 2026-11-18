// Long idle/rev soak of the real-time session; writes raw stereo f32 for analysis.
// usage: node live_long.mjs <engine> <quality 0|1> <seconds> <out.f32> [pattern idle|blips]
import fs from 'node:fs';
const [engine, q, secs, outPath, pattern = 'idle'] = process.argv.slice(2);
const { instance } = await WebAssembly.instantiate(fs.readFileSync(new URL('../live/engsim.wasm', import.meta.url)), {});
const x = instance.exports;
const req = new TextEncoder().encode(JSON.stringify({ engine, overrides: {} }));
const p = x.es_alloc(req.length); new Uint8Array(x.memory.buffer, p, req.length).set(req);
const l = x.es_new(p, req.length, +q, 0);
const out = x.es_alloc(128 * 8); const fd = fs.openSync(outPath, 'w');
const blocks = Math.round(+secs * 375);
for (let b = 0; b < blocks; b++) {
  const t = b / 375;
  if (pattern === 'blips') x.es_set_throttle(l, (t % 6) < 0.6 ? 0.7 : 0);
  x.es_process(l, out, 128);
  fs.writeSync(fd, Buffer.from(new Uint8Array(x.memory.buffer, out, 128 * 8)));
}
fs.closeSync(fd); console.log(engine, 'rpm at end', x.es_rpm(l).toFixed(0));
