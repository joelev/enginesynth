// Stand-in for live/engine.js with the same interface. It replays a rendered telemetry
// directory (telemetry.json + telemetry.bin + mix.wav) in a loop: frames follow the audio
// clock, the throttle and workshop settings are ignored. Used with ?mock=1, or automatically
// when the real engine module is not there.

const DIRS = {
  v8_cross: 'swap_cross_live96W',
  v8_flat: 'swap_flat_live96W',
  flat6: 'flat6_pull',
  i4: 'lineup_i4',
  boxer4_uel: 'lineup_boxer4_uel',
  v10_f1: 'lineup_v10_f1',
  v12: 'lineup_v12',
  vtwin45: 'lineup_vtwin45',
  single: 'lineup_single',
  wankel2: 'wankel2_revs',
  wankel4_pp: 'wankel4_pp_revs',
};
const ROOT = new URL('../out.nosync/', import.meta.url).href;
const urlOf = (name) => {
  const q = new URLSearchParams(location.search).get('data');
  return q && name === '__data' ? q.replace(/\/$/, '') : ROOT + DIRS[name];
};

async function loadSet(ctx, name) {
  const base = urlOf(name);
  const [hr, br, wr] = await Promise.all([fetch(`${base}/telemetry.json`), fetch(`${base}/telemetry.bin`), fetch(`${base}/mix.wav`)]);
  if (!hr.ok || !br.ok) throw new Error(`no telemetry for ${name} at ${base}`);
  const header = await hr.json();
  const bin = new Float32Array(await br.arrayBuffer());
  const audio = wr.ok ? await ctx.decodeAudioData(await wr.arrayBuffer()) : null;
  return { header, bin, audio };
}

export class LiveEngine extends EventTarget {
  static isMock = true;

  static async presets() {
    const out = [];
    await Promise.all(Object.keys(DIRS).map(async (name) => {
      try {
        const r = await fetch(`${urlOf(name)}/telemetry.json`);
        if (!r.ok) return;
        const s = (await r.json()).spec;
        out.push({ kind: s.layout === 'rotary' ? 'wankel' : 'piston', units: s.rotor_phase ? s.rotor_phase.length : s.firing.length, name, label: s.label, cylinders: s.firing.length, layout: s.layout, redline: s.redline, idle: s.idle, itb: s.itb, litres: Math.PI / 4 * s.bore * s.bore * s.stroke * s.firing.length * 1000, spec: s });
      } catch (e) { /* that set is not on disk */ }
    }));
    const order = Object.keys(DIRS);
    return out.sort((a, b) => order.indexOf(a.name) - order.indexOf(b.name));
  }

  static async create(audioCtx, { engine = 'v8_cross' } = {}) {
    const e = new LiveEngine(audioCtx);
    await e.setEngine(engine);
    return e;
  }

  constructor(ctx) {
    super();
    this.ctx = ctx;
    this.output = ctx.createGain();
    this.stats = { load: 0, quality: 'replay', dropped: 0 };
    this.header = null;
    this.frame = null;
    this._src = null;
    this._t0 = 0;
    this._timer = setInterval(() => this._tick(), 1000 / 120);
  }

  async setEngine(name) {
    const set = await loadSet(this.ctx, DIRS[name] ? name : 'v8_cross');
    if (this._src) { try { this._src.stop(); } catch (e) { /* not started */ } this._src.disconnect(); }
    this._set = set;
    this.name = name;
    const FF = set.header.frame_floats;
    this._n = Math.min(set.header.frames, Math.floor(set.bin.length / FF));
    this._times = new Float32Array(this._n);
    for (let i = 0; i < this._n; i++) this._times[i] = set.bin[i * FF];
    this._dur = set.audio ? set.audio.duration : this._times[this._n - 1];
    this.header = set.header;
    this._start(0);
    this._tick(true);
    this.dispatchEvent(new Event('rebuild'));
  }

  _start(at) {
    if (this._src) { try { this._src.stop(); } catch (e) { /* not started */ } this._src.disconnect(); }
    this._src = null;
    if (this._set.audio) {
      const s = this.ctx.createBufferSource();
      s.buffer = this._set.audio;
      s.loop = true;
      s.connect(this.output);
      s.start(0, at % this._dur);
      this._src = s;
    }
    this._t0 = this.ctx.currentTime - at;
  }

  /** Jump the replay to a time in seconds (for tests and screenshots). */
  seek(t) {
    this._start(t);
    this._tick(true);
  }

  _tick(force) {
    if (!this._set) return;
    const t = ((this.ctx.currentTime - this._t0) % this._dur + this._dur) % this._dur;
    let lo = 0, hi = this._n - 1;
    while (lo < hi) { const m = (lo + hi + 1) >> 1; if (this._times[m] <= t) lo = m; else hi = m - 1; }
    if (lo === this._i && !force) return;
    this._i = lo;
    const FF = this.header.frame_floats;
    this.frame = this._set.bin.subarray(lo * FF, (lo + 1) * FF);
    this.dispatchEvent(new Event('frame'));
  }

  setThrottle() {}
  setBrake() {}
  setMode() {}
  setAutoShift() {}
  shift() {}
  setMixture() {}
  setOverrunFuel() {}
  setLiftoffPops() {}
  setLimiter() {}
  get mixture() { return 1; }
  get overrunFuel() { return 0; }
  get limiter() { return 'fuel'; }
}
