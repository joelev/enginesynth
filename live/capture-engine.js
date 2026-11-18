// Offline capture (?capture=1; used by tools/render_live_video.py, not offered anywhere in the page).
//
// The same simulator, but run on the main thread and advanced only when asked: step(n) computes n
// samples (800 for one video frame at 60 fps), keeps them, and takes the newest telemetry frame,
// which the page then draws exactly as it draws a live one. Nothing here runs in real time, so a
// video made from the frames and the kept samples is in step by construction.
//
// It has the LiveEngine interface the page uses (live/README.md). Sound is not played: `output` is
// a silent node. The page's two readers of the sound (the oscilloscope's analyser and the
// spectrum for the spectrogram widget and the floor, live/spectro.js) are replaced by `tap`, which
// computes from the kept samples what a Web Audio AnalyserNode would have given: the last fftSize
// samples mixed to mono, a Blackman window, magnitudes divided by the size, smoothed over calls
// with the node's time constant, in decibels.

import { LiveEngine } from './engine.js';
import { SpectroSource } from './spectro.js';

const QUALITY = ['live48+W+m+r', 'live96+W+m+r', 'live48', 'live96'];
const text = (x, p, n) => new TextDecoder().decode(new Uint8Array(x.memory.buffer, p, n));
const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
let exportsPromise = null;
const wasm = () => exportsPromise || (exportsPromise = fetch(new URL('./engsim.wasm', import.meta.url)).then((r) => r.arrayBuffer()).then((b) => WebAssembly.instantiate(b, {})).then((m) => m.instance.exports));

/** In-place radix-2 FFT of a complex signal (re, im), length a power of two. */
function fft(re, im) {
  const n = re.length;
  for (let i = 1, j = 0; i < n; i++) {
    let bit = n >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) { let t = re[i]; re[i] = re[j]; re[j] = t; t = im[i]; im[i] = im[j]; im[j] = t; }
  }
  for (let len = 2; len <= n; len <<= 1) {
    const ang = -2 * Math.PI / len, wr = Math.cos(ang), wi = Math.sin(ang);
    for (let i = 0; i < n; i += len) {
      let cr = 1, ci = 0;
      for (let k = 0; k < len / 2; k++) {
        const a = i + k, b = a + len / 2;
        const xr = re[b] * cr - im[b] * ci, xi = re[b] * ci + im[b] * cr;
        re[b] = re[a] - xr; im[b] = im[a] - xi; re[a] += xr; im[a] += xi;
        const t = cr * wr - ci * wi; ci = cr * wi + ci * wr; cr = t;
      }
    }
  }
}

/** What an AnalyserNode of this size and smoothing would report, computed from the kept samples. */
class Analyser {
  constructor(tap, fftSize, smoothing) {
    this.tap = tap; this.fftSize = fftSize; this.frequencyBinCount = fftSize / 2; this.smoothingTimeConstant = smoothing;
    this.win = new Float32Array(fftSize);
    for (let i = 0; i < fftSize; i++) this.win[i] = 0.42 - 0.5 * Math.cos(2 * Math.PI * i / fftSize) + 0.08 * Math.cos(4 * Math.PI * i / fftSize);
    this.re = new Float32Array(fftSize); this.im = new Float32Array(fftSize);
    this.smooth = new Float32Array(fftSize / 2);
  }
  getFloatTimeDomainData(out) { this.tap.last(out); }
  getFloatFrequencyData(out) {
    const n = this.fftSize, re = this.re, im = this.im, k = this.smoothingTimeConstant;
    this.tap.last(re);
    for (let i = 0; i < n; i++) { re[i] *= this.win[i]; im[i] = 0; }
    fft(re, im);
    for (let i = 0; i < n / 2; i++) {
      const s = this.smooth[i] = k * this.smooth[i] + (1 - k) * Math.hypot(re[i], im[i]) / n;
      out[i] = s > 0 ? 20 * Math.log10(s) : -Infinity;
    }
  }
}

/** The page's SpectroSource (live/spectro.js), fed from the kept samples instead of two AnalyserNodes. */
class Spectrum {
  constructor(tap, rate) {
    this.rate = rate;
    this.lo = new Analyser(tap, 8192, 0.5);
    this.hi = new Analyser(tap, 2048, 0.35);
    this.bLo = new Float32Array(this.lo.frequencyBinCount);
    this.bHi = new Float32Array(this.hi.frequencyBinCount);
  }
  read() { this.lo.getFloatFrequencyData(this.bLo); this.hi.getFloatFrequencyData(this.bHi); }
}
// column() is SpectroSource's own, so the mapping from spectra to picture is one piece of code
Spectrum.prototype.column = SpectroSource.prototype.column;

export class CaptureEngine extends EventTarget {
  static presets = () => LiveEngine.presets();
  static measure = (e, o, q) => LiveEngine.measure(e, o, q);
  static preview = (e, o) => LiveEngine.preview(e, o);
  static slow = 1;

  static async create(audioCtx, opts = {}) {
    const e = new CaptureEngine(audioCtx, await wasm(), opts.quality || 'auto');
    await e.setEngine(opts.engine || 'v8_cross', opts.overrides || {});
    return e;
  }
  constructor(audioCtx, x, quality) {
    super();
    this.audioCtx = audioCtx; this.x = x;
    this.output = audioCtx.createGain();      // silent: nothing feeds it
    this.header = null; this.frame = null; this.l = 0; this.seq = -1;
    this.stats = { load: 0, quality: '', dropped: 0 };
    this.engine = ''; this.overrides = {};
    this._qualityPref = quality;
    this._throttle = 0; this._drive = false; this._auto = true;
    this._phi = 1; this._overrunFuel = 0; this._limiter = 'fuel'; this._liftoffPops = 0;
    this._blowoff = null; this._whistle = null; // turbo engines only; null = the engine's own setting
    this.cap = 1024; this.out = x.es_alloc(this.cap * 2 * 4);
    this.gain = 0.8;                          // the page's master gain, ahead of its analysers
    this.chunks = []; this.samples = 0;       // everything computed since record(): stereo, interleaved
    this.recording = false;
    this.ring = new Float32Array(16384); this.ringAt = 0;   // the newest mono samples (after the gain), for the readers
    const tap = this;
    this.tap = { analyser: new Analyser({ last: (o) => tap.last(o) }, 4096, 0.55), spectrum: null };
    this.tap.spectrum = new Spectrum({ last: (o) => tap.last(o) }, 48000);
  }
  /** The newest out.length mono samples, oldest first. */
  last(out) {
    const n = out.length, N = this.ring.length;
    for (let i = 0; i < n; i++) out[i] = this.ring[(this.ringAt - n + i + 2 * N) % N];
  }

  async setEngine(name, overrides = {}) {
    const x = this.x;
    // offline there is time for the finer level; 'light' is honoured when asked for
    const q = this._qualityPref === 'light' ? 0 : 1;
    const req = new TextEncoder().encode(JSON.stringify({ engine: name, overrides: overrides || {} }));
    const p = x.es_alloc(req.length); new Uint8Array(x.memory.buffer, p, req.length).set(req);
    const startRpm = (this.frame && this.engine === name) ? this.frame[2] : 0;
    const l = x.es_new(p, req.length, q, startRpm); x.es_free(p, req.length);
    if (!l) throw new Error('engine request rejected');
    if (this.l) x.es_drop(this.l);
    this.l = l; this.seq = -1;
    if (this._drive) x.es_set_drive(l, 1);
    if (!this._auto) x.es_set_auto(l, 0);
    x.es_set_throttle(l, this._throttle);
    x.es_set_phi(l, this._phi); x.es_set_overrun_fuel(l, this._overrunFuel); x.es_set_limiter(l, this._limiter === 'spark' ? 1 : 0); x.es_set_liftoff_pops(l, this._liftoffPops);
    if (this._blowoff !== null && x.es_set_blowoff) x.es_set_blowoff(l, { none: 0, recirc: 1, vent: 2 }[this._blowoff]);
    if (this._whistle !== null && x.es_set_whistle) x.es_set_whistle(l, this._whistle);
    this.engine = name; this.overrides = overrides;
    this.header = JSON.parse(text(x, x.es_header_ptr(l), x.es_header_len(l))); this.frame = null;
    this.stats.quality = this.header.quality;
    this.dispatchEvent(new Event('rebuild'));
    return this.header;
  }
  get level() { return this.header && this.header.quality === QUALITY[1] ? 'heavy' : 'light'; }
  get qualityPref() { return this._qualityPref; }
  async setQuality(pref, rebuild = true) { this._qualityPref = pref === 'heavy' || pref === 'light' ? pref : 'auto'; if (rebuild && this.engine) return this.setEngine(this.engine, this.overrides); }

  /** Advance the simulator by n samples (in blocks of 128, as the audio thread does) and take the newest telemetry frame. */
  step(n) {
    const x = this.x, l = this.l;
    if (!l) return;
    const keep = this.recording ? new Float32Array(2 * n) : null;
    for (let done = 0; done < n;) {
      const m = Math.min(128, n - done);
      x.es_process(l, this.out, m);
      const v = new Float32Array(x.memory.buffer, this.out, 2 * m);
      if (keep) keep.set(v, 2 * done);
      for (let i = 0; i < m; i++) { this.ring[this.ringAt] = this.gain * 0.5 * (v[2 * i] + v[2 * i + 1]); this.ringAt = (this.ringAt + 1) % this.ring.length; }
      done += m;
      const seq = x.es_frame_seq(l);
      if (seq !== this.seq) { this.seq = seq; this.frame = new Float32Array(x.memory.buffer, x.es_frame_ptr(l), x.es_frame_len(l)).slice(); }
    }
    if (keep) { this.chunks.push(keep); this.samples += n; }
    this.dispatchEvent(new Event('frame'));
  }
  /** Start keeping the samples (dropping any kept before). */
  record() { this.chunks = []; this.samples = 0; this.recording = true; }
  /** The kept samples, stereo interleaved, as computed (before the page's master gain). */
  recorded() { const a = new Float32Array(2 * this.samples); let o = 0; for (const c of this.chunks) { a.set(c, o); o += c.length; } return a; }

  setThrottle(v) { this._throttle = clamp(v, 0, 1); if (this.l) this.x.es_set_throttle(this.l, this._throttle); }
  setBrake(v) { if (this.l) this.x.es_set_brake(this.l, clamp(v, 0, 1)); }
  setMode(m) { this._drive = m === 'drive'; if (this.l) this.x.es_set_drive(this.l, this._drive ? 1 : 0); if (this._drive && !this._auto) this.setAutoShift(false); }
  setAutoShift(b) { this._auto = !!b; if (this.l) this.x.es_set_auto(this.l, this._auto ? 1 : 0); }
  shift(dir) { if (this.l) this.x.es_shift(this.l, dir > 0 ? 1 : -1); }
  setMixture(phi) { const v = Number(phi); if (!Number.isFinite(v)) return; this._phi = clamp(v, 0.7, 1.7); if (this.l) this.x.es_set_phi(this.l, this._phi); }
  setOverrunFuel(v) { v = Number(v); if (!Number.isFinite(v)) return; this._overrunFuel = clamp(v, 0, 1); if (this.l) this.x.es_set_overrun_fuel(this.l, this._overrunFuel); }
  setLiftoffPops(v) { v = Number(v); if (!Number.isFinite(v)) return; this._liftoffPops = clamp(v, 0, 1); if (this.l) this.x.es_set_liftoff_pops(this.l, this._liftoffPops); }
  setLimiter(kind) { this._limiter = kind === 'spark' ? 'spark' : 'fuel'; if (this.l) this.x.es_set_limiter(this.l, this._limiter === 'spark' ? 1 : 0); }
  get mixture() { return this._phi; }
  get overrunFuel() { return this._overrunFuel; }
  get limiter() { return this._limiter; }
  get liftoffPops() { return this._liftoffPops; }
  // Turbo blow-off routing ('vent', 'recirc', 'none') and whistle stand-in gain, while running (no-ops on other engines).
  setBlowoff(mode) { if (!['vent', 'recirc', 'none'].includes(mode)) return; this._blowoff = mode; if (this.l && this.x.es_set_blowoff) this.x.es_set_blowoff(this.l, { none: 0, recirc: 1, vent: 2 }[mode]); }
  setWhistle(v) { v = Number(v); if (!Number.isFinite(v)) return; this._whistle = clamp(v, 0, 16); if (this.l && this.x.es_set_whistle) this.x.es_set_whistle(this.l, this._whistle); }
  get blowoff() { return this._blowoff; }
  get whistle() { return this._whistle; }
}
export { CaptureEngine as LiveEngine };
