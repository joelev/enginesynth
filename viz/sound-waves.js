// The sound leaving the engine, drawn from the simulation's own numbers.
//
// Every telemetry frame carries, for each radiating opening (tailpipe, intake snorkel, open
// throttle), the last 128 samples of the sound it sent out (pascals at 1 m, newest first, at
// header.sound.rate). In the model each opening is a point source in free air, so the pressure
// at distance r from it now is hist(r / c) / r: the sample that left r / c seconds ago, divided
// by r. The field of all openings is the sum of theirs.
//
// Each frame, for every opening, this module turns its history into a radial profile: the
// pressure p(r) at 512 radii out to the furthest drawn, with its first and second derivatives
// in r. That is where the whole data-to-radius mapping lives (age-to-radius curve, display
// smoothing, 1/r spreading), in JavaScript. The profiles go to the GPU as one small float
// texture (one row per opening); a shader on the floor sums them at each point's true 3D
// distance from every opening and colours the result:
//
//   - compression warm, rarefaction cool, brightness a power law of pressure, and
//   - brightness weighted toward the wavefronts (crests and troughs), so it reads as rings.
//
// Everything that turns data into picture is in `waveMapping` below; change it there, or at run
// time from the console (window.__app.scene.waves.mapping), and the next frame uses it.

import * as THREE from 'three';

/** The mapping from sound data to picture. Read every frame, so it can be changed live. */
export const waveMapping = {
  /** Age-to-radius curve: how far from its opening (metres) the sound that left `age` seconds
   *  ago is drawn. `c` is the speed of sound from the header, `t` the frame's engine time (s).
   *  Must increase with age. Default: the physics, radius = age x c. */
  radiusOfAge: (age, c, t) => age * c,
  /** Brightness is a power law of pressure (Stevens-style, as loudness and brightness are
   *  perceived): y = (|p| / pFull)^exponent, then softly limited, b = y / (1 + y) x gain.
   *  With exponent 0.45 a pressure 40 dB lower is drawn about 8 times dimmer, so an idling
   *  engine is faint but there and a race engine's pulses stand out. pFull is in pascals at the
   *  drawn point, after the 1/r spreading. */
  pFull: 300,
  exponent: 0.45,
  /** Overall brightness (linear colour). The page's bloom starts near 1.5. */
  gain: 1.6,
  /** Rarefaction is drawn at this fraction of the brightness of the same-sized compression. */
  negGain: 0.85,
  /** Wavefronts: brightness is weighted toward the crests and troughs of the pressure along the
   *  floor, so the field reads as rings rather than a carpet. crestLen (m) is the width of a
   *  ring (widened to a pixel and a half where the floor is far away); fill is the share of
   *  brightness kept between rings (1 = no weighting, the plain field). */
  crestLen: 0.05,
  fill: 0.12,
  /** Rings are drawn only where the pressure curves like a wave shorter than this (m); a broad
   *  swell is left as fill. */
  maxWave: 2.5,
  /** Display smoothing of each history before it is drawn (a Gaussian low-pass), so the rings are
   *  the firing pulses and the pipes' strong resonances rather than every small ripple. Given
   *  the frame's firing frequency (Hz, all cylinders or chambers), returns the -3 dB frequency;
   *  0 or Infinity draws the raw data. Default: 2.5 x the firing frequency, at least 600 Hz,
   *  which keeps the firing pulse and its second harmonic at every engine speed. */
  smoothHz: (fireHz) => Math.max(600, 2.5 * fireHz),
  /** Closer than this (m) the 1/r spreading is held at 1/rMin: the point-source model means
   *  nothing inside the size of the opening, and the value would run away. */
  rMin: 0.08,
  /** Outer fade: brightness falls to zero between fadeFrom x (furthest radius) and that radius. */
  fadeFrom: 0.5,
  /** Tints for compression and rarefaction (linear RGB, multiplied by brightness). */
  warm: [1.0, 0.50, 0.16],
  cool: [0.22, 0.52, 1.0],
};

const MAXS = 32;    // most openings one engine can have drawn (uniform array size)
const NR = 512;     // radii in each opening's profile

const FLOOR_VERT = `
varying vec3 vW;
void main() { vec4 w = modelMatrix * vec4(position, 1.0); vW = w.xyz; gl_Position = projectionMatrix * viewMatrix * w; }`;
const FLOOR_FRAG = `
uniform sampler2D uProf;     // RGBA32F, NR x openings: p, dp/dr, d2p/dr2 at radius i / (NR - 1) x uRMax
uniform vec4 uSrc[${MAXS}];  // xyz = opening position (w unused)
uniform int uCount;
uniform float uRMax, uFadeFrom, uPFull, uExp, uGain, uNegGain, uCrestLen, uFill, uMaxWave;
uniform vec3 uWarm, uCool;
varying vec3 vW;
void main() {
  // sum of every opening's field at this point; gradient and curvature along the floor
  float sum = 0.0, win = 0.0;
  vec2 grad = vec2(0.0);
  mat2 hess = mat2(0.0);
  for (int j = 0; j < ${MAXS}; j++) {
    if (j >= uCount) break;
    vec3 d = vW - uSrc[j].xyz;
    float r = length(d);
    if (r >= uRMax) continue;
    float x = r / uRMax * float(${NR - 1});
    int i = int(x);
    vec3 q = mix(texelFetch(uProf, ivec2(i, j), 0).rgb, texelFetch(uProf, ivec2(min(i + 1, ${NR - 1}), j), 0).rgb, x - float(i));
    vec2 u = d.xz / max(r, 1e-4);           // the radial direction, seen in the floor
    float rr = max(r, 1e-3);
    sum += q.x;
    grad += q.y * u;
    // Hessian of p(r) restricted to the floor: p'' u u^T + p' / r (I - u u^T)
    hess += q.z * outerProduct(u, u) + (q.y / rr) * (mat2(1.0) - outerProduct(u, u));
    win = max(win, 1.0 - smoothstep(uFadeFrom * uRMax, uRMax, r));
  }
  // distance to the nearest crest (or trough, where the pressure is negative) along the floor:
  // one Newton step, slope / curvature; only where it curves like a wave shorter than uMaxWave
  float gl = length(grad);
  vec2 gh = grad / max(gl, 1e-9);
  float curv = -sign(sum) * dot(gh, hess * gh);
  float k = sqrt(max(curv, 0.0) / max(abs(sum), 1e-6)), kMin = 6.2832 / uMaxWave;
  float gate = smoothstep(0.6 * kMin, kMin, k);
  float dist = gate > 0.0 ? gl / curv / gate : 1e9;
  float px = length(fwidth(vW.xz));
  float w = max(uCrestLen, 1.5 * px);
  float line = (1.0 - smoothstep(0.5 * w, w, dist)) * min(1.0, uCrestLen / w);
  float y = pow(abs(sum) / uPFull, uExp);
  float b = y / (1.0 + y) * win * uGain * mix(line, 1.0, uFill);
  gl_FragColor = vec4(sum >= 0.0 ? uWarm * b : uCool * (b * uNegGain), 1.0);
}`;

/** Sound-wave picture for one engine model. `opts.floorY` is the floor height (m),
 *  `opts.layer` the render layer for its meshes. Draws nothing when the header has no sound block. */
export class SoundWaves {
  constructor(model, opts = {}) {
    const H = model.H, S = H.sound;
    this.mapping = opts.mapping || waveMapping;
    this.root = new THREE.Group();
    this.root.name = 'sound-waves';
    this.sources = (model.soundSources || []).filter((s) => s.pos && s.index < MAXS);
    this.enabled = !!(S && S.count > 0 && this.sources.length);
    if (!this.enabled) return;
    this.S = S;
    this.samples = S.samples;
    this.rate = S.rate;
    this.c = S.speed_of_sound || 343;
    this.offset = S.offset;
    this.rows = Math.min(S.count, MAXS);
    // firings per crank (shaft) revolution: every cylinder or chamber once per cycle
    this.firePerRev = H.spec.firing.length * 360 / (H.cycle_deg || 720);
    this.hist = new Float32Array(this.samples * S.count);     // this frame's histories, smoothed
    this.prof = new Float32Array(NR * this.rows * 4);          // what the GPU gets
    this.profTex = new THREE.DataTexture(this.prof, NR, this.rows, THREE.RGBAFormat, THREE.FloatType);
    this.profTex.minFilter = this.profTex.magFilter = THREE.NearestFilter;
    this.age = new Float32Array(NR);      // age (samples) drawn at each profile radius
    this.dage = new Float32Array(NR);     // its rate of change with radius (samples per metre)
    this.fwd = new Float32Array(4 * this.samples);
    // a row with no drawn opening is parked far away, out of reach
    const src = [];
    for (let j = 0; j < MAXS; j++) src.push(new THREE.Vector4(1e6, 1e6, 1e6, 0));
    for (const s of this.sources) src[s.index].set(s.pos.x, s.pos.y, s.pos.z, 0);
    this.uniforms = {
      uProf: { value: this.profTex }, uSrc: { value: src }, uCount: { value: this.rows },
      uRMax: { value: 1 }, uFadeFrom: { value: 0 }, uPFull: { value: 1 }, uExp: { value: 1 }, uGain: { value: 1 }, uNegGain: { value: 1 },
      uCrestLen: { value: 1 }, uFill: { value: 1 }, uMaxWave: { value: 1 }, uWarm: { value: new THREE.Vector3() }, uCool: { value: new THREE.Vector3() },
    };
    // floor: a unit disc under the openings, scaled every frame to the furthest radius drawn
    const cx = this.sources.reduce((a, s) => a + s.pos.x, 0) / this.sources.length;
    const cz = this.sources.reduce((a, s) => a + s.pos.z, 0) / this.sources.length;
    this.floorSpan = Math.max(...this.sources.map((s) => Math.hypot(s.pos.x - cx, s.pos.z - cz)));
    this.floorMesh = new THREE.Mesh(new THREE.CircleGeometry(1, 96), new THREE.ShaderMaterial({
      uniforms: this.uniforms, vertexShader: FLOOR_VERT, fragmentShader: FLOOR_FRAG,
      transparent: true, depthWrite: false, blending: THREE.AdditiveBlending, fog: false,
    }));
    this.floorMesh.rotation.x = -Math.PI / 2;
    this.floorMesh.position.set(cx, (opts.floorY ?? -0.5) + 0.004, cz);
    this.floorMesh.frustumCulled = false;
    this.root.add(this.floorMesh);
    if (opts.layer !== undefined) this.root.traverse((o) => o.layers.set(opts.layer));
    this.t = 0;
    this.applyMapping();
  }

  /** Read `mapping`: the radius-to-age table and the shader's colour settings. Run every frame. */
  applyMapping() {
    const m = this.mapping, u = this.uniforms, N = this.samples, fwd = this.fwd, K = fwd.length;
    // forward: radius at ages 0, 1/4, 2/4 ... (N - 1) samples; then its inverse at the profile radii
    for (let k = 0; k < K; k++) fwd[k] = m.radiusOfAge((k / 4) / this.rate, this.c, this.t);
    const rMax = fwd[K - 1];
    let k = 0;
    for (let i = 0; i < NR; i++) {
      const r = rMax * i / (NR - 1);
      while (k < K - 2 && fwd[k + 1] < r) k++;
      const d = fwd[k + 1] - fwd[k];
      this.age[i] = Math.min(N - 1, (k + (d > 0 ? Math.min(1, Math.max(0, (r - fwd[k]) / d)) : 0)) / 4);
      this.dage[i] = d > 0 ? 0.25 / d : 0;
    }
    this.rMax = rMax;
    u.uRMax.value = rMax; u.uFadeFrom.value = m.fadeFrom; u.uPFull.value = m.pFull; u.uExp.value = m.exponent;
    u.uGain.value = m.gain; u.uNegGain.value = m.negGain; u.uCrestLen.value = m.crestLen; u.uFill.value = m.fill; u.uMaxWave.value = m.maxWave;
    u.uWarm.value.set(...m.warm); u.uCool.value.set(...m.cool);
    const s = this.floorSpan + rMax + 0.05;
    this.floorMesh.scale.set(s, s, 1);
  }

  /** Take this frame's sound histories. `bin` holds the frame at offset `o`. */
  update(bin, o = 0) {
    if (!this.enabled || !this.root.visible) return;
    const a = o + this.offset, n = this.hist.length;
    if (bin.length < a + n) return;
    this.t = bin[o];
    this.applyMapping();
    // display smoothing: Gaussian sigma (samples) for the -3 dB frequency, sigma_t = 0.1325 / f
    const fc = this.mapping.smoothHz(bin[o + 2] / 60 * this.firePerRev);
    this.sigma = fc > 0 && Number.isFinite(fc) ? 0.1325 * this.rate / fc : 0;
    smoothRows(bin.subarray(a, a + n), this.hist, this.samples, this.sigma);
    this.buildProfiles();
    this.profTex.needsUpdate = true;
  }

  /** p(r) = h(age(r)) / max(r, rMin) for each opening, with dp/dr and d2p/dr2: the history is
   *  read with Catmull-Rom interpolation (smooth first derivative), the age table is piecewise
   *  linear (its own second derivative is taken as zero). */
  buildProfiles() {
    const N = this.samples, P = this.prof, rMin = this.mapping.rMin;
    for (let j = 0; j < this.rows; j++) {
      const row = j * N;
      for (let i = 0; i < NR; i++) {
        const r = this.rMax * i / (NR - 1), ag = this.age[i], da = this.dage[i];
        const s = Math.floor(ag), t = ag - s, at = (q) => this.hist[row + Math.min(Math.max(q, 0), N - 1)];
        const p0 = at(s - 1), p1 = at(s), p2 = at(s + 1), p3 = at(s + 2);
        const c1 = -p0 + p2, c2 = 2 * p0 - 5 * p1 + 4 * p2 - p3, c3 = -p0 + 3 * p1 - 3 * p2 + p3;
        const h = 0.5 * (2 * p1 + t * (c1 + t * (c2 + t * c3)));
        const h1 = 0.5 * (c1 + t * (2 * c2 + 3 * t * c3)) * da, h2 = (c2 + 3 * t * c3) * da * da;
        const R = Math.max(r, rMin), out = r > rMin;
        const v = h / R;
        const v1 = h1 / R - (out ? v / R : 0);
        const v2 = h2 / R - (out ? 2 * h1 / (R * R) - 2 * v / (R * R) : 0);
        const q = 4 * (j * NR + i);
        P[q] = v; P[q + 1] = v1; P[q + 2] = v2; P[q + 3] = 0;
      }
    }
  }

  setVisible(on) { this.root.visible = !!on; }

  /** Pressure (Pa) along a line out from opening `j` this frame: [radius, p] pairs read from the
   *  profile uploaded to the GPU, interpolated the way the shader does. Used by the checks. */
  radialProfile(j, step = 0.005) {
    const out = [];
    for (let r = 0; r < this.rMax; r += step) {
      const x = r / this.rMax * (NR - 1), i = Math.floor(x), q = 4 * (j * NR + i), q1 = 4 * (j * NR + Math.min(i + 1, NR - 1));
      out.push([r, this.prof[q] + (this.prof[q1] - this.prof[q]) * (x - i)]);
    }
    return out;
  }

  dispose() {
    if (this.enabled) {
      this.floorMesh.geometry.dispose(); this.floorMesh.material.dispose();
      this.profTex.dispose();
    }
    if (this.root.parent) this.root.parent.remove(this.root);
  }
}

/** Gaussian smoothing of each `N`-sample row of `src` into `dst` (edges held), sigma in samples. */
function smoothRows(src, dst, N, sigma) {
  if (!(sigma > 0)) { dst.set(src); return; }
  const R = Math.ceil(3 * sigma), w = [];
  let tot = 0;
  for (let k = -R; k <= R; k++) { w.push(Math.exp(-0.5 * k * k / (sigma * sigma))); tot += w[w.length - 1]; }
  for (let o = 0; o < src.length; o += N) {
    for (let i = 0; i < N; i++) {
      let acc = 0;
      for (let k = -R; k <= R; k++) acc += w[k + R] * src[o + Math.min(N - 1, Math.max(0, i + k))];
      dst[o + i] = acc / tot;
    }
  }
}
