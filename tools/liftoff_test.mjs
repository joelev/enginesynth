// Lift-off pops through the page's own JavaScript, outside a browser: live/engine.js (LiveEngine)
// talking to live/worklet.js (the audio processor) over a stand-in message port, on the real
// WebAssembly build. Numeric checks only; nobody listens here.
// usage: node tools/liftoff_test.mjs
import fs from 'node:fs';
const module = await WebAssembly.compile(fs.readFileSync(new URL('../live/engsim.wasm', import.meta.url)));

// the two globals an AudioWorkletGlobalScope gives worklet.js
let Processor = null;
const procPort = { peer: null, onmessage: null, queue: [], postMessage(m) { this.peer ? this.peer.onmessage({ data: m }) : this.queue.push(m); } };
globalThis.sampleRate = 48000;
globalThis.AudioWorkletProcessor = class { constructor() { this.port = procPort; } };
globalThis.registerProcessor = (name, cls) => { Processor = cls; };
await import('../live/worklet.js');
const { LiveEngine } = await import('../live/engine.js');

const proc = new Processor({ processorOptions: { module } });
const nodePort = { onmessage: null, postMessage(m) { procPort.onmessage({ data: m }); } };
const node = { port: nodePort };
const eng = new LiveEngine({ sampleRate: 48000 }, node, 'light');
procPort.peer = nodePort;
for (const m of procPort.queue) nodePort.onmessage({ data: m });
await eng._ready;

let fails = 0;
const check = (name, ok, detail = '') => { console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ': ' + detail : ''}`); if (!ok) fails++; };

// 1. the setter: clamps to 0..1, ignores what is not a number, posts one message each time
const sent = []; const post = nodePort.postMessage.bind(nodePort);
nodePort.postMessage = (m) => { sent.push(m); post(m); };
check('default is 0', eng.liftoffPops === 0);
eng.setLiftoffPops(0.3); eng.setLiftoffPops(7); eng.setLiftoffPops(-2); eng.setLiftoffPops(NaN); eng.setLiftoffPops('x');
check('setLiftoffPops clamps and ignores non-numbers', JSON.stringify(sent.map(m => [m.type, m.value])) === JSON.stringify([['liftoffPops', 0.3], ['liftoffPops', 1], ['liftoffPops', 0]]) && eng.liftoffPops === 0, JSON.stringify(sent));

// 2. a blip to the limiter and a lift, at 0 and at 1: flashes in the afterfire telemetry
const outL = new Float32Array(128), outR = new Float32Array(128);
async function blip(level, engine) {
  eng.setLiftoffPops(level);
  const header = await eng.setEngine(engine);      // the 'new' message carries the setting
  const af = header.afterfire;
  let flashes = 0, prev = 0, bad = 0, peak = 0, firstT = -1, lastT = -1, maxGlow = 0, frames = 0;
  const onFrame = () => { frames++; };
  eng.addEventListener('frame', onFrame);
  const total = Math.round(6.5 * 48000 / 128);
  for (let b = 0; b < total; b++) {
    const t = b * 128 / 48000;
    if (b === Math.round(0.3 * 375)) eng.setThrottle(1);
    if (b === Math.round(2.0 * 375)) eng.setThrottle(0);
    proc.process([], [[outL, outR]]);
    for (let i = 0; i < 128; i++) { if (!Number.isFinite(outL[i]) || !Number.isFinite(outR[i])) bad++; peak = Math.max(peak, Math.abs(outL[i])); }
    if (eng.frame) {
      let g = 0; for (let k = 0; k < af.count; k++) g = Math.max(g, eng.frame[af.offset + k]);
      if (g > prev + 0.02) { flashes++; if (firstT < 0) firstT = t - 2.0; lastT = t - 2.0; }   // a new or renewed flash
      prev = g; maxGlow = Math.max(maxGlow, g);
    }
  }
  eng.removeEventListener('frame', onFrame);
  return { header, flashes, bad, peak, firstT, lastT, maxGlow, frames, rpm: eng.frame[2], floats: eng.frame.length };
}
for (const engine of ['v8_cross', 'i4']) {
  const a = await blip(0, engine), b = await blip(1, engine), c = await blip(0, engine);
  console.log(`${engine}: level 0: ${a.flashes} flashes, peak glow ${a.maxGlow.toFixed(2)}; level 1: ${b.flashes} flashes between ${b.firstT.toFixed(2)} and ${b.lastT.toFixed(2)} s after the lift, peak glow ${b.maxGlow.toFixed(2)}, output peak ${b.peak.toFixed(2)}; rpm at the end ${a.rpm.toFixed(0)} / ${b.rpm.toFixed(0)} (idle ${a.header.spec.idle})`);
  check(`${engine}: nothing at 0, before and after a run at 1`, a.flashes === 0 && a.maxGlow === 0 && c.flashes === 0);
  check(`${engine}: a handful of flashes at 1, all after the lift and within 2 s of it`, b.flashes >= 2 && b.flashes <= 14 && b.firstT > 0 && b.lastT < 2.0);
  check(`${engine}: finite, frame layout unchanged by the setting, back at idle`, a.bad + b.bad + c.bad === 0 && a.floats === b.floats && a.header.frame_floats === b.header.frame_floats && Math.abs(b.rpm / a.header.spec.idle - 1) < 0.15);
}
// 3. changing it while running reaches the simulator (no rebuild): set 1 mid-run, lift, expect flashes
{
  await blip(0, 'i4');
  const header = eng.header, af = header.afterfire; let flashes = 0, prev = 0;
  for (let b = 0; b < 6 * 375; b++) {
    if (b === 100) { eng.setThrottle(1); eng.setLiftoffPops(1); }
    if (b === 800) eng.setThrottle(0);
    proc.process([], [[outL, outR]]);
    if (eng.frame) { let g = 0; for (let k = 0; k < af.count; k++) g = Math.max(g, eng.frame[af.offset + k]); if (g > prev + 0.02) flashes++; prev = g; }
  }
  check('set while running takes effect without a rebuild', flashes >= 1, `${flashes} flashes`);
}
if (fails) { console.error(`${fails} check(s) failed`); process.exit(1); }
