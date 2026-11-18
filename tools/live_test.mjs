// Exercise the real-time session the way the audio worklet will, outside a browser.
import fs from 'node:fs';
const { instance } = await WebAssembly.instantiate(fs.readFileSync(new URL('../live/engsim.wasm', import.meta.url)), {});
const x = instance.exports;
const str = (p, n) => new TextDecoder().decode(new Uint8Array(x.memory.buffer, p, n));
const presets = JSON.parse(str(x.es_presets_ptr(), x.es_presets_len()));
console.log('presets:', presets.map(p => p.name).join(' '));
function make(engine, quality, overrides = {}) {
  const req = new TextEncoder().encode(JSON.stringify({ engine, overrides }));
  const p = x.es_alloc(req.length); new Uint8Array(x.memory.buffer, p, req.length).set(req);
  const l = x.es_new(p, req.length, quality, 0); x.es_free(p, req.length);
  return l;
}
const out = x.es_alloc(128 * 2 * 4);
function run(l, secs, each) {
  const blocks = Math.round(secs * 48000 / 128); let peak = 0, bad = 0; const t0 = performance.now();
  for (let b = 0; b < blocks; b++) {
    if (each) each(b * 128 / 48000);
    x.es_process(l, out, 128);
    const v = new Float32Array(x.memory.buffer, out, 256);
    for (let i = 0; i < 256; i++) { if (!Number.isFinite(v[i])) bad++; peak = Math.max(peak, Math.abs(v[i])); }
  }
  return { load: (performance.now() - t0) / 1000 / secs, peak, bad };
}
for (const [name, q] of [['v8_cross', 0], ['v8_cross', 1], ['flat6', 1], ['i4', 1], ['v10_f1', 0], ['single', 1]]) {
  const l = make(name, q); const h = JSON.parse(str(x.es_header_ptr(l), x.es_header_len(l)));
  const idle = run(l, 1.0);
  const rpmIdle = x.es_rpm(l);
  const blip = run(l, 1.5, t => x.es_set_throttle(l, t < 0.5 ? 0.6 : 0));
  const f = new Float32Array(x.memory.buffer, x.es_frame_ptr(l), x.es_frame_len(l));
  console.log(`${name.padEnd(9)} ${h.quality.padEnd(9)} idle load ${idle.load.toFixed(2)} rpm ${rpmIdle.toFixed(0)} | blip load ${blip.load.toFixed(2)} peak ${blip.peak.toFixed(2)} bad ${idle.bad + blip.bad} | frames ${x.es_frame_seq(l)} floats ${f.length}=${h.frame_floats} rpm-in-frame ${f[2].toFixed(0)}`);
  x.es_drop(l);
}
// drive mode: pull away from rest, shift up, lift, brake
const l = make('flat6', 0); x.es_set_drive(l, 1); let log = [];
run(l, 14, t => { x.es_set_throttle(l, t < 1 ? 0 : t < 9 ? 1 : 0); x.es_set_brake(l, t > 11 ? 0.7 : 0);
  if (Math.abs(t * 2 - Math.round(t * 2)) < 0.0014) { const f = new Float32Array(x.memory.buffer, x.es_frame_ptr(l), 8); log.push(`${t.toFixed(1)}s ${f[2].toFixed(0)}rpm g${f[5]} ${(f[6] * 3.6).toFixed(0)}km/h`); } });
console.log('drive:', log.filter((_, i) => i % 2 == 0).join(' | '));
// rebuild with overrides
const l2 = make('i4', 1, { primary_len: [0.9, 0.9, 0.9, 0.9], muffler_len: 0 }); console.log('override build ok:', l2 !== 0, run(l2, 0.5).bad === 0);
console.log('bad request returns null:', make('nope', 0) === 0);
// fueling controls: rich mixture, fueled overrun, spark-cut limiter; afterfire must show in the telemetry
{
  const l = make('v8_cross', 0); const h = JSON.parse(str(x.es_header_ptr(l), x.es_header_len(l)));
  const af = h.afterfire; let glow = 0, glowDefault = 0;
  const peekGlow = () => { const f = new Float32Array(x.memory.buffer, x.es_frame_ptr(l), x.es_frame_len(l)); let g = 0; for (let k = 0; k < af.count; k++) g = Math.max(g, f[af.offset + k]); return g; };
  // defaults: a blip into the limiter and a lift shows no afterfire
  const d = run(l, 3, t => { x.es_set_throttle(l, t > 0.3 && t < 1.8 ? 1 : 0); glowDefault = Math.max(glowDefault, peekGlow()); });
  x.es_set_phi(l, 1.3); x.es_set_overrun_fuel(l, 1); x.es_set_limiter(l, 1);
  const r = run(l, 3, t => { x.es_set_throttle(l, t > 0.3 && t < 1.8 ? 1 : 0); glow = Math.max(glow, peekGlow()); });
  x.es_set_phi(l, 99); x.es_set_phi(l, NaN); x.es_set_overrun_fuel(l, -3);
  const r2 = run(l, 0.5);
  console.log(`fueling: header afterfire ${JSON.stringify(af)} (floats ${h.frame_floats}); defaults: peak glow ${glowDefault.toFixed(2)}, bad ${d.bad}; phi 1.3 + overrun fuel 1 + spark cut: peak glow ${glow.toFixed(2)}, peak ${r.peak.toFixed(2)}, bad ${r.bad}; out-of-range inputs bad ${r2.bad}`);
  console.log('afterfire field last in frame:', af.offset + af.count === h.frame_floats, '| no afterfire at defaults:', glowDefault === 0, '| afterfire with fueling:', glow > 0);
  x.es_drop(l);
}
