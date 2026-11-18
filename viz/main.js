// engsim viewer: draws one telemetry directory (telemetry.json + telemetry.bin + mix.wav).
// One telemetry frame is one video frame. Nothing here assumes frames are evenly spaced in engine time.
//
// URL parameters
//   data=<url of the telemetry directory>      capture=1 (no UI; frames are driven by window.renderFrame)
//   az0, az1 (deg), el (deg), dist (scale), tx, ty, tz (m), focus=all|engine, fov (deg)
//   caption=<text>   bloom=<strength>   pfull=<bar at full colour, default 0.5>   hud=0   grid=0
//   waves=0 (no sound waves; they are drawn only when the telemetry has a sound block)
//   wavemap=<JSON overriding fields of waveMapping in sound-waves.js>

import * as THREE from 'three';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js';
import { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';
import { RoomEnvironment } from 'three/addons/environments/RoomEnvironment.js';
import { EngineModel, config, pressureColor } from './engine-model.js';
import { SoundWaves, waveMapping } from './sound-waves.js';

const Q = new URLSearchParams(location.search);
const num = (k, d) => (Q.has(k) && Q.get(k) !== '' && isFinite(+Q.get(k)) ? +Q.get(k) : d);
const DATA = (Q.get('data') || '../out.nosync/test_i4').replace(/\/$/, '');
const CAPTURE = Q.get('capture') === '1';
const V3 = THREE.Vector3;
const RAD = Math.PI / 180;
const $ = (id) => document.getElementById(id);
config.pFull = num('pfull', config.pFull);
config.pExp = num('pexp', config.pExp);
config.gain = num('gain', config.gain);
const P_FULL = config.pFull;

// (colour maps, layout and meshes live in engine-model.js)

// ---------------------------------------------------------------- data loading
async function loadWav(url) {
  try {
    const res = await fetch(url);
    if (!res.ok) return null;
    const buf = await res.arrayBuffer(), dv = new DataView(buf);
    let o = 12, ch = 2, rate = 48000, bits = 16, fmt = 1;
    while (o + 8 <= dv.byteLength) {
      const id = String.fromCharCode(dv.getUint8(o), dv.getUint8(o + 1), dv.getUint8(o + 2), dv.getUint8(o + 3));
      const size = dv.getUint32(o + 4, true);
      if (id === 'fmt ') { fmt = dv.getUint16(o + 8, true); ch = dv.getUint16(o + 10, true); rate = dv.getUint32(o + 12, true); bits = dv.getUint16(o + 22, true); }
      if (id === 'data') {
        const nfr = Math.floor(Math.min(size, dv.byteLength - o - 8) / (ch * bits / 8));
        const mono = new Float32Array(nfr);
        for (let i = 0; i < nfr; i++) {
          let s = 0;
          for (let c = 0; c < ch; c++) {
            const q = o + 8 + (i * ch + c) * (bits / 8);
            s += fmt === 3 ? dv.getFloat32(q, true) : dv.getInt16(q, true) / 32768;
          }
          mono[i] = s / ch;
        }
        return { mono, rate };
      }
      o += 8 + size + (size & 1);
    }
  } catch (e) { /* no audio is fine */ }
  return null;
}

// ---------------------------------------------------------------- main
async function main() {
  const H = await (await fetch(`${DATA}/telemetry.json`)).json();
  const bin = new Float32Array(await (await fetch(`${DATA}/telemetry.bin`)).arrayBuffer());
  const FF = H.frame_floats, NF = Math.min(H.frames, Math.floor(bin.length / FF));
  if (!NF) throw new Error('telemetry has no frames');
  const wav = Q.get('audio') === '0' ? null : await loadWav(`${DATA}/mix.wav`);
  let extra = {};
  try { const r = await fetch(`${DATA}/viz.json`); if (r.ok) extra = await r.json(); } catch (e) { /* optional */ }
  const sp = H.spec, n = sp.firing.length;

  const canvas = $('gl');
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, preserveDrawingBuffer: true, powerPreference: 'high-performance' });
  renderer.setPixelRatio(CAPTURE ? 1 : Math.min(2, window.devicePixelRatio));
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = num('exposure', 1.0);
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x05060a);
  const pm = new THREE.PMREMGenerator(renderer);
  scene.environment = pm.fromScene(new RoomEnvironment(), 0.04).texture;
  scene.environmentIntensity = num('env', 0.42);
  const key = new THREE.DirectionalLight(0xfff4e6, 1.15);
  key.position.set(-1.6, 2.6, 2.4);
  scene.add(key);
  const rimLight = new THREE.DirectionalLight(0x9fc0ff, 1.5);
  rimLight.position.set(2.5, 1.4, -2.6);
  scene.add(rimLight);
  const fill = new THREE.DirectionalLight(0xffffff, 0.25);
  fill.position.set(-2, -0.5, -1);
  scene.add(fill);

  const model = new EngineModel(H);
  scene.add(model.root);
  const { L, crank, cyls, rods, tubes, vols, boxEng, boxAll, ptsCore, ptsEng, ptsAll, state } = model;
  // ---- camera. Named views; az0/az1/el/dist/tx/ty/tz/focus override the preset.
  const coreC = boxEng.getCenter(new V3());
  for (const q of ptsCore) coreC.add(q);
  coreC.addScaledVector(boxEng.getCenter(new V3()), -1).multiplyScalar(1 / 8);
  const engLen = crank.x1 - crank.x0;
  const VIEWS = {
    // engine about a third of the frame height, exhaust trailing out to the right
    engine: { pts: ptsCore, target: coreC.clone().add(new V3(0.22 * engLen, 0, 0)), az: [-46, -30], el: 14, bx: 0.66, by: [-0.34, 0.40] },
    wide: { pts: ptsAll, target: boxAll.getCenter(new V3()), az: [-38, -20], el: 20, bx: 0.86, by: [-0.56, 0.66] },
    // from the tailpipe end, looking back up the pipes
    exhaust: { pts: ptsAll, target: boxAll.getCenter(new V3()), az: [64, 50], el: 13, bx: 0.84, by: [-0.5, 0.62] },
    // end-on, along the crank
    front: model.kind === 'wankel'
      ? { pts: ptsCore, target: coreC.clone(), az: [-100, -80], el: 4, bx: 0.5, by: [-0.42, 0.46] }
      : { pts: ptsEng, target: boxEng.getCenter(new V3()), az: [-97, -83], el: 5, bx: 0.8, by: [-0.5, 0.62] },
  };
  let view = VIEWS[Q.get('view')] || VIEWS.engine;
  if (Q.get('focus') === 'all') view = VIEWS.wide;
  if (Q.get('focus') === 'engine') view = { ...VIEWS.wide, pts: ptsEng, target: boxEng.getCenter(new V3()) };
  const fov = num('fov', 30);
  const camera = new THREE.PerspectiveCamera(fov, 16 / 9, 0.02, 60);
  const target = view.target.clone().add(new V3(num('tx', 0), num('ty', 0), num('tz', 0)));
  const az0 = num('az0', view.az[0]), az1 = num('az1', view.az[1]), el = num('el', view.el);
  const placeCamera = (azDeg, d) => {
    const az = azDeg * RAD, e = el * RAD;
    camera.position.set(target.x + d * Math.sin(az) * Math.cos(e), target.y + d * Math.sin(e), target.z + d * Math.cos(az) * Math.cos(e));
    camera.lookAt(target);
    camera.updateMatrixWorld(true);
  };
  // smallest distance at which the framed points stay inside the view's bounds
  const fits = (d) => [az0, az1, 0.5 * (az0 + az1)].every((az) => {
    placeCamera(az, d);
    return view.pts.every((c) => { const q = c.clone().project(camera); return Math.abs(q.x) < view.bx && q.y > view.by[0] && q.y < view.by[1] && q.z < 1; });
  });
  let dLo = 0.2, dHi = 40;
  for (let k = 0; k < 30; k++) { const m = 0.5 * (dLo + dHi); if (fits(m)) dHi = m; else dLo = m; }
  const camDist = dHi * num('dist', 1);
  if (Q.get('grid') !== '0') {
    const grid = new THREE.GridHelper(12, 120, 0x1c2330, 0x10141c);
    grid.position.y = boxAll.min.y - 0.06;
    scene.add(grid);
    scene.fog = new THREE.Fog(0x05060a, camDist * 1.5, camDist * 5);
  }
  // sound leaving the openings, when the telemetry carries it (older telemetry: nothing drawn)
  // wavemap={"gain":0.8,...} overrides fields of waveMapping (viz/sound-waves.js) for this render
  if (Q.get('wavemap')) Object.assign(waveMapping, JSON.parse(Q.get('wavemap')));
  const waves = Q.get('waves') === '0' ? null : new SoundWaves(model, { floorY: boxAll.min.y - 0.06 });
  if (waves && waves.enabled) scene.add(waves.root);
  window.waves = waves;

  const composer = new EffectComposer(renderer);
  composer.addPass(new RenderPass(scene, camera));
  const bloom = new UnrealBloomPass(new THREE.Vector2(1920, 1080), num('bloom', 0.32), 0.5, 1.6);
  composer.addPass(bloom);
  composer.addPass(new OutputPass());
  function resize() {
    const w = window.innerWidth, h = window.innerHeight;
    renderer.setSize(w, h, false);
    composer.setSize(w, h);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    $('hud').style.transform = `scale(${w / 1920})`;
  }
  window.addEventListener('resize', resize);
  resize();

  const AC = Q.get('pmode') === 'ac';
  // ---- HUD (static parts)
  if (Q.get('hud') === '0') $('hud').style.display = 'none';
  $('label').textContent = sp.label;
  const dispL = Math.PI / 4 * sp.bore * sp.bore * sp.stroke * n * 1000;
  if (H.kind === 'wankel') {
    // displacement of one chamber; the litre figure is the usual four-stroke equivalent (one chamber per rotor)
    const cc = 3 * Math.sqrt(3) * sp.ecc * sp.rotor_r * sp.rotor_w * 1e6, nr = H.units;
    $('sub').textContent = `${(nr * cc / 1000).toFixed(1)} L (${nr} × ${Math.round(cc)} cc) · ${nr} rotors · eccentricity ${(sp.ecc * 1000).toFixed(0)} mm`;
    $('firingcap').textContent = 'combustion, per chamber (three per rotor)';
  } else $('sub').textContent = `${dispL.toFixed(dispL < 2 ? 2 : 1)} L · ${n} cyl · bore ${(sp.bore * 1000).toFixed(1)} mm × stroke ${(sp.stroke * 1000).toFixed(1)} mm`;
  const lamps = [];
  for (let i = 0; i < n; i++) {
    const d = document.createElement('div');
    d.className = 'cyl';
    const lamp = document.createElement('div');
    lamp.className = 'lamp';
    d.append(lamp, String(i + 1));
    $('firing').appendChild(d);
    lamps.push(lamp);
  }
  const css = (c) => `rgb(${c.map((v) => Math.round(255 * Math.pow(Math.min(1, v / (1 + 0.25 * v)), 1 / 2.2))).join(',')})`;
  const stops = [];
  for (let k = 0; k <= 20; k++) {
    const dp = (k / 10 - 1) * P_FULL, c = [0, 0, 0];
    pressureColor(dp, c, 0);
    stops.push(`${css(c)} ${k * 5}%`);
  }
  $('legendbar').style.background = `linear-gradient(90deg, ${stops.join(',')})`;
  if (AC) $('legendcap').textContent = 'gas pressure minus its local running average, bar';
  // boosted engines: the charge air has its own teal scale (engine-model.js chargeColor)
  if (model.chargeLook) $('legendcap').textContent += ` · charge air: teal, full at +${+model.chargeLook.pFull.toFixed(2)} bar`;
  $('tag').textContent = Q.get('tag') || extra.tag || '';
  $('lgneg').textContent = `−${P_FULL}`;
  $('lgpos').textContent = `+${P_FULL}`;
  const tachMax = Math.ceil(sp.redline * 1.08 / 1000) * 1000;
  $('tachred').style.width = `${100 * (1 - sp.redline / tachMax)}%`;
  const stepK = tachMax > 12000 ? 4 : tachMax > 6000 ? 2 : 1;
  for (let k = 0; k <= tachMax / 1000; k += stepK) { const s = document.createElement('span'); s.textContent = k; $('tachticks').appendChild(s); }
  const SCOPE_MS = 60;
  const scope = $('scope'), sctx = scope.getContext('2d');
  if (!wav) $('scopebox').style.display = 'none';
  $('scopewin').textContent = `${SCOPE_MS} ms window`;
  const captionOf = (i) => {
    if (Q.get('caption')) return Q.get('caption');
    if (Array.isArray(extra.captions)) for (const c of extra.captions) if (i >= c.from && i <= c.to) return c.text;
    return extra.caption || '';
  };

  function drawScope(t) {
    const W = scope.width, Hh = scope.height;
    sctx.clearRect(0, 0, W, Hh);
    sctx.strokeStyle = '#1b212c';
    sctx.lineWidth = 2;
    sctx.beginPath(); sctx.moveTo(0, Hh / 2); sctx.lineTo(W, Hh / 2); sctx.stroke();
    if (!wav) return;
    const half = SCOPE_MS * 0.0005 * wav.rate, c0 = t * wav.rate;
    sctx.strokeStyle = '#e9edf4';
    sctx.lineWidth = 2.5;
    sctx.beginPath();
    for (let x = 0; x < W; x += 2) {
      const a = Math.floor(c0 - half + (x / W) * 2 * half), b = Math.floor(c0 - half + ((x + 2) / W) * 2 * half);
      let lo = 1, hi = -1;
      for (let s = a; s <= b; s++) { const v = s >= 0 && s < wav.mono.length ? wav.mono[s] : 0; if (v < lo) lo = v; if (v > hi) hi = v; }
      const y0 = Hh / 2 - hi * Hh * 0.48, y1 = Hh / 2 - lo * Hh * 0.48;
      if (x === 0) sctx.moveTo(x, y0); else sctx.lineTo(x, y0);
      if (y1 - y0 > 1) sctx.lineTo(x, y1);
    }
    sctx.stroke();
  }

  // ---- per-frame update
  let pref = null;
  if (AC) {
    // running average of each pressure sample over a window of frames (about one engine cycle in strobe data)
    const half = Math.max(1, Math.round(num('acwin', 61) / 2));
    pref = new Float32Array(NF * FF);
    const cols = [];
    for (const tb of tubes) if (tb) for (let j = 0; j < tb.pipe.samples; j++) cols.push(tb.pipe.offset + 3 * j);
    for (const v of vols) if (v) cols.push(v.offset);
    const acc = new Float64Array(NF + 1);
    for (const c of cols) {
      for (let i = 0; i < NF; i++) acc[i + 1] = acc[i] + bin[i * FF + c];
      for (let i = 0; i < NF; i++) { const a = Math.max(0, i - half), b = Math.min(NF, i + half + 1); pref[i * FF + c] = (acc[b] - acc[a]) / (b - a); }
    }
  }
  const refP = pref ? (o, c) => pref[o + c] : null;
  function update(i) {
    const o = i * FF;
    const t = bin[o], rpm = bin[o + 2], thr = bin[o + 3], gear = bin[o + 5], speed = bin[o + 6], cut = bin[o + 7];
    model.update(bin, o, refP);
    if (waves) waves.update(bin, o);
    for (let k = 0; k < n; k++) {
      const lit = Math.min(1, bin[o + H.cyl_offset + 6 * k + 4] / 1.6);
      lamps[k].style.background = `rgba(255,${Math.round(150 + 90 * lit)},${Math.round(60 + 150 * lit)},${lit.toFixed(3)})`;
      lamps[k].style.boxShadow = lit > 0.05 ? `0 0 ${Math.round(14 * lit)}px rgba(255,170,70,${(0.8 * lit).toFixed(3)})` : 'none';
    }
    const f = NF > 1 ? i / (NF - 1) : 0;
    placeCamera(az0 + (az1 - az0) * f, camDist);
    // HUD
    $('rpm').firstChild.nodeValue = String(Math.round(rpm / 10) * 10);
    $('tachfill').style.width = `${Math.min(100, 100 * rpm / tachMax)}%`;
    $('thrfill').style.width = `${Math.min(100, 100 * thr)}%`;
    $('gear').textContent = gear > 0 ? `GEAR ${Math.round(gear)}   ${Math.round(speed * 3.6)} km/h` : '';
    $('cut').style.visibility = cut > 0.5 ? 'visible' : 'hidden';
    $('caption').textContent = captionOf(i);
    drawScope(t);
  }
  function renderFrame(i) {
    const k = Math.min(NF - 1, Math.max(0, i | 0));
    update(k);
    composer.render();
    return k;
  }

  // ---- consistency checks (run by tools/render_video.py --verify)
  function verifyRotary(step) {
    const out = { kind: 'wankel', frames: NF, apexToBore: 0, chamberCentreDeg: 0, minFlankClearance: Infinity, nan: 0, lengths: [] };
    for (let i = 0; i < NF; i += step) {
      update(i);
      scene.updateMatrixWorld(true);
      model.checkRotary(bin, i * FF + H.cyl_offset, out);
    }
    tubes.forEach((t, k) => out.lengths.push(t ? { name: t.pipe.name, physical: t.pipe.len, shown: +t.shown.toFixed(4) } : { name: H.pipes[k].name, missing: true }));
    out.sound = model.checkSoundSources();
    scene.traverse((obj) => {
      const pa = obj.geometry && obj.geometry.attributes.position;
      if (pa) for (let q = 0; q < pa.array.length; q++) if (!Number.isFinite(pa.array[q])) out.nan++;
      if (obj.isObject3D) for (const v of [obj.position.x, obj.position.y, obj.position.z, obj.rotation.x, obj.scale.y]) if (!Number.isFinite(v)) out.nan++;
    });
    return out;
  }
  function verify(step = 1) {
    if (model.kind === 'wankel') return verifyRotary(step);
    const out = { frames: NF, rodPin: 0, rodWrist: 0, primaryStart: 0, primaryEnd: 0, runnerEnd: 0, runnerStart: 0, nan: 0, minPrimarySpacing: Infinity, minOpenPipeGap: Infinity, minCrownToHead: Infinity, minValveToCrown: Infinity, clearanceHeight: L.hc, lengths: [] };
    const a = new V3(), b = new V3();
    state.clear = Infinity; state.valveClear = Infinity;
    for (let i = 0; i < NF; i += step) {
      update(i);
      scene.updateMatrixWorld(true);
      for (let k = 0; k < n; k++) {
        crank.pinMarkers[k].getWorldPosition(a);
        rods[k].group.getWorldPosition(b);
        out.rodPin = Math.max(out.rodPin, a.distanceTo(b));
        rods[k].tip.getWorldPosition(a);
        cyls[k].piston.getWorldPosition(b);
        out.rodWrist = Math.max(out.rodWrist, a.distanceTo(b));
      }
    }
    // valve-to-piston clearance over the whole cycle, from the spec's own valve events
    out.cycleMinValveToCrown = L.two ? null : L.hc + L.recess + L.minGap;   // null: no valves (two-stroke)
    if (L.two) {
      // the drawn ports open when the drawn crown passes their top edge: check that against the timing
      const P = L.ports, sp = H.spec;
      const dist = (deg) => { const c = deg * Math.PI / 180; return L.r * Math.cos(c) + Math.sqrt(L.l * L.l - L.r * L.r * Math.sin(c) * Math.sin(c)); };
      out.exPortEdgeErr = Math.abs(dist(sp.ex_port_open) + L.ch - P.exTop);
      out.trPortEdgeErr = Math.abs(dist(sp.tr_port_open) + L.ch - P.trTop);
      out.portsBelowTDCcrown = P.exTop < L.deck - L.hc && P.trTop < P.exTop;
    }
    out.valvePocketDepth = L.recess;
    out.flatChamberWouldGive = L.hcFull + L.minGap;
    out.minCrownToHead = state.clear;
    out.minValveToCrown = state.valveClear;
    out.sound = model.checkSoundSources();
    tubes.forEach((t, k) => {
      if (!t) { out.lengths.push({ name: H.pipes[k].name, missing: true }); return; }
      const p = t.pipe, c = L.paths[k];
      out.lengths.push({ name: p.name, physical: p.len, shown: +t.shown.toFixed(4) });
      if (p.kind === 'primary') {
        cyls[p.cyl].exMark.getWorldPosition(a);
        out.primaryStart = Math.max(out.primaryStart, a.distanceTo(c.getPointAt(0)));
        if (p.right.type === 'volume') {
          const inlet = L.primEnd[p.cyl], cs = L.volShape[p.right.index];
          out.primaryEnd = Math.max(out.primaryEnd, inlet.distanceTo(c.getPointAt(1)), Math.max(0, inlet.distanceTo(vols[p.right.index].mesh.position) - cs.ring - 1e-9));
        } else {
          // open-ended pipes (expansion chambers) of different cylinders: smallest gap between their surfaces
          const rad = (q, u) => { const S = q.samples, f = Math.min(S - 1, u * S), i0 = Math.floor(f), i1 = Math.min(S - 1, i0 + 1); return 0.5 * (q.dia[i0] + (q.dia[i1] - q.dia[i0]) * (f - i0)); };
          tubes.forEach((t2, k2) => {
            if (!t2 || k2 <= k || t2.pipe.kind !== 'primary' || t2.pipe.right.type === 'volume') return;
            for (let u = 0; u <= 1.0001; u += 0.01) for (let w = 0; w <= 1.0001; w += 0.01) {
              const gap = c.getPointAt(Math.min(1, u)).distanceTo(L.paths[k2].getPointAt(Math.min(1, w))) - rad(p, u) - rad(t2.pipe, w);
              if (gap < out.minOpenPipeGap) { out.minOpenPipeGap = gap; out.minOpenPipeAt = `${p.name}@${u.toFixed(2)} ${t2.pipe.name}@${w.toFixed(2)}`; }
            }
          });
        }
        // closest approach between this primary and the others of its group, in pipe radii (below 2 means they overlap)
        tubes.forEach((t2, k2) => {
          if (!t2 || k2 <= k || t2.pipe.kind !== 'primary' || p.right.type !== 'volume' || t2.pipe.right.index !== p.right.index) return;
          let dmin = Infinity;
          let at = '';
          for (let u = 0; u <= 1.0001; u += 0.02) for (let w = 0; w <= 1.0001; w += 0.02) {
            const dd = c.getPointAt(Math.min(1, u)).distanceTo(L.paths[k2].getPointAt(Math.min(1, w)));
            if (dd < dmin) { dmin = dd; at = `${p.name}@${u.toFixed(2)} ${t2.pipe.name}@${w.toFixed(2)}`; }
          }
          if (dmin / (0.5 * p.dia[0]) < out.minPrimarySpacing) { out.minPrimarySpacing = dmin / (0.5 * p.dia[0]); out.minPrimaryAt = at; }
        });
      }
      if (p.kind === 'runner') {
        cyls[p.cyl].inMark.getWorldPosition(a);
        out.runnerEnd = Math.max(out.runnerEnd, a.distanceTo(c.getPointAt(1)));
        const vm = vols[p.left.index].mesh.position;
        const s = L.volShape[p.left.index];
        // runners start on the plenum body (within its radius of the axis) or at their throttle volume
        const dAxis = s.type === 'capsule' ? Math.hypot(c.getPointAt(0).y - vm.y, c.getPointAt(0).z - vm.z) - s.r : c.getPointAt(0).distanceTo(vm) - 0.0151;
        out.runnerStart = Math.max(out.runnerStart, Math.max(0, dAxis));
      }
    });
    scene.traverse((obj) => {
      const pa = obj.geometry && obj.geometry.attributes.position;
      if (pa) for (let q = 0; q < pa.array.length; q++) if (!Number.isFinite(pa.array[q])) out.nan++;
      if (obj.isObject3D) for (const v of [obj.position.x, obj.position.y, obj.position.z, obj.rotation.x, obj.scale.y]) if (!Number.isFinite(v)) out.nan++;
    });
    return out;
  }

  const times = new Float32Array(NF);
  for (let i = 0; i < NF; i++) times[i] = bin[i * FF];
  const gl = renderer.getContext(), dbg = gl.getExtension('WEBGL_debug_renderer_info');
  window.sceneInfo = { frames: NF, t0: times[0], t1: times[NF - 1], rate: H.rate, label: sp.label, gl: dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : 'unknown' };
  window.frameTimes = () => Array.from(times);
  window.renderFrame = renderFrame;
  window.verifyScene = verify;
  renderFrame(num('frame', 0));

  // ---- interactive playback (plain static server, no capture flag)
  if (!CAPTURE) {
    $('ui').style.display = 'flex';
    const seek = $('seek'), btn = $('play');
    seek.max = NF - 1;
    const span = times[NF - 1] - times[0], video = (NF - 1) / 60;
    const realtime = wav && video > 0 && Math.abs(span - video) / video < 0.05;
    const audio = realtime ? new Audio(`${DATA}/mix.wav`) : null;
    $('uiinfo').textContent = realtime ? '' : 'frames are not real time (slow motion or stroboscopic): audio off';
    let playing = false, cur = 0, wall0 = 0, f0 = 0;
    const frameAt = (t) => { let lo = 0, hi = NF - 1; while (lo < hi) { const m = (lo + hi + 1) >> 1; if (times[m] <= t) lo = m; else hi = m - 1; } return lo; };
    function tick(now) {
      if (playing) {
        cur = audio ? frameAt(audio.currentTime) : (f0 + Math.floor((now - wall0) * 0.06)) % NF;
        if (audio && (audio.ended || cur >= NF - 1)) { playing = false; btn.textContent = 'play'; }
        seek.value = cur;
        renderFrame(cur);
      }
      requestAnimationFrame(tick);
    }
    btn.onclick = () => {
      playing = !playing;
      btn.textContent = playing ? 'pause' : 'play';
      if (playing) {
        if (cur >= NF - 1) cur = 0;
        wall0 = performance.now(); f0 = cur;
        if (audio) { audio.currentTime = times[cur]; audio.play(); }
      } else if (audio) audio.pause();
    };
    seek.oninput = () => { cur = +seek.value; f0 = cur; wall0 = performance.now(); if (audio) audio.currentTime = times[cur]; renderFrame(cur); };
    requestAnimationFrame(tick);
  }
  window.sceneReady = true;
}

main().catch((e) => {
  window.sceneError = String(e && e.stack || e);
  $('err').textContent = window.sceneError;
});
