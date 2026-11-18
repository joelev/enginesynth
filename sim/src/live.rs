//! Real-time session: one engine, stepped a block at a time for an audio callback, with
//! stroboscopic telemetry frames for the picture. Exposed to WebAssembly through a C ABI.

use crate::audio::Decim;
use crate::engine::*;
use crate::gas::*;
use crate::prof::{self, cat};
use crate::spec::{self, EngineSpec};
use std::collections::VecDeque;
use std::f64::consts::PI;

const FRAME_RATE: f64 = 60.0;
/// The browser uses 0 (light) and 1 (heavy). 5 and up are experiments:
/// `+f` f32 grid fluxes, `+m` cheaper cylinder maths, `+r` warm-started radiation solve.
// The two levels the page uses (0 light, 1 heavy) include `+m` and `+r` (about 11% less CPU,
// differences at rounding level). The plain names stay at 13 and 14.
const QUALITIES: [&str; 15] = ["live48+W+m+r", "live96+W+m+r", "live48", "live96", "live48+w", "live48+W+f", "live96+W+f", "live48+W+m", "live96+W+m", "live48+W+f+m", "live96+W+f+m", "live48+W+m+r", "live96+W+m+r", "live48+W", "live96+W"];

struct Pending {
    frame: usize,
    phase: f64,
    data: Vec<f32>,
}

pub struct Live {
    sys: System,
    spec: EngineSpec,
    k: usize,
    rate: f64,
    dec: [Decim; 4],
    samples: Vec<usize>,
    header: Vec<u8>,
    frame: Vec<f32>,
    scratch: Vec<f32>,
    frame_seq: u32,
    pending: VecDeque<Pending>,
    created: usize,
    last_phase: f64,
    prev_theta: f64,
    inner: u64,
    env: f64,
    level: f64,
    fade: f64,
}

impl Live {
    pub fn new(req: &serde_json::Value, quality_idx: usize, start_rpm: f64) -> Option<Live> {
        let name = req.get("engine")?.as_str()?;
        let mut s = spec::preset(name)?;
        if let Some(map) = req.get("overrides").and_then(|o| o.as_object()) {
            let mut v = serde_json::to_value(&s).ok()?;
            for (key, val) in map {
                if v.get(key).is_some() {
                    v[key] = val.clone();
                }
            }
            s = serde_json::from_value::<EngineSpec>(v).ok()?.finish().ok()?;
        }
        let qname = QUALITIES[quality_idx % QUALITIES.len()];
        let q = crate::quality(qname);
        let k = (q.rate / 48_000.0).round() as usize;
        let mut sys = System::build(&s, &q, 0x9E3779B97F4A7C15);
        sys.mode = Mode::Free;
        if start_rpm > 0.0 {
            sys.omega = start_rpm * PI / 30.0;
        }
        let samples: Vec<usize> = sys.pipes.iter().map(|p| ((p.len * 40.0).round() as usize).clamp(4, 400).min(p.n)).collect();
        let mut frame = Vec::new();
        sys.snapshot(&samples, &mut frame);
        let header = header_json(&sys, &s, &samples, frame.len(), qname, q.rate);
        let prev_theta = sys.theta;
        Some(Live {
            sys,
            spec: s,
            k,
            rate: q.rate,
            dec: [Decim::new(k), Decim::new(k), Decim::new(k), Decim::new(k)],
            samples,
            header: serde_json::to_vec(&header).ok()?,
            frame,
            scratch: Vec::new(),
            frame_seq: 0,
            pending: VecDeque::new(),
            created: 0,
            last_phase: 0.0,
            prev_theta,
            inner: 0,
            env: 0.0,
            level: 4.0,
            fade: 0.0,
        })
    }

    fn strobe(&mut self) {
        let t = self.inner as f64 / self.rate;
        let four_pi = self.spec.cycle_deg() * PI / 180.0;
        let rpm = self.sys.rpm().max(60.0);
        let look = ((FRAME_RATE * 120.0 / rpm).ceil() as usize + 1).clamp(2, 14);
        let now_frame = (t * FRAME_RATE).floor() as usize;
        while self.created <= now_frame + look {
            let step = (9.0 + 17.0 * (rpm / self.spec.redline).min(1.0)).min(rpm * 6.0 / FRAME_RATE);
            self.last_phase = (self.last_phase + step * PI / 180.0).rem_euclid(four_pi);
            let mut data = Vec::with_capacity(self.frame.len());
            self.sys.snapshot(&self.samples, &mut data);
            self.pending.push_back(Pending { frame: self.created, phase: self.last_phase, data });
            self.created += 1;
        }
        let theta = self.sys.theta;
        let mut hit: Option<usize> = None;
        for (i, p) in self.pending.iter().enumerate() {
            let a = (self.prev_theta - p.phase).rem_euclid(four_pi);
            let b = (theta - p.phase).rem_euclid(four_pi);
            if b < a {
                hit = Some(i);
                break; // phases are distinct; at most one crossing matters per step
            }
        }
        if let Some(i) = hit {
            let mut data = std::mem::take(&mut self.pending[i].data);
            data.clear();
            self.sys.snapshot(&self.samples, &mut data);
            self.pending[i].data = data;
        }
        while let Some(front) = self.pending.front() {
            if t < front.frame as f64 / FRAME_RATE {
                break;
            }
            let mut p = self.pending.pop_front().unwrap();
            self.scratch.clear();
            self.sys.snapshot(&self.samples, &mut self.scratch);
            let (af_off, af_n) = self.sys.afterfire_layout(&self.samples);
            let (bo_off, bo_n) = self.sys.boost_layout(&self.samples);
            let bo_n = bo_n * System::BOOST_FIELDS.len() + self.sys.turbos.len() * System::TURBO_FIELDS.len();
            for gi in [0usize, 2, 3, 4, 5, 6, 7].into_iter().chain(af_off..af_off + af_n).chain(bo_off..bo_off + bo_n) {
                p.data[gi] = self.scratch[gi];
            }
            if p.data.len() == self.frame.len() {
                self.frame.copy_from_slice(&p.data);
                self.frame_seq = self.frame_seq.wrapping_add(1);
            }
        }
        self.prev_theta = theta;
    }

    /// Fill `out` (interleaved stereo, 48 kHz) with the next block.
    pub fn process(&mut self, out: &mut [f32]) {
        let (att, rel) = (1.0 - (-1.0f64 / (0.010 * 48_000.0)).exp(), 1.0 - (-1.0f64 / (0.250 * 48_000.0)).exp());
        for fr in out.chunks_exact_mut(2) {
            let mut s = [0.0f64; 4];
            for _ in 0..self.k {
                let o = self.sys.advance();
                self.inner += 1;
                let qt = prof::now();
                self.strobe();
                let qt = prof::lap(cat::STROBE, qt);
                for c in 0..4 {
                    if let Some(v) = self.dec[c].push(o[c]) {
                        s[c] = v;
                    }
                }
                prof::lap(cat::DECIM, qt);
            }
            let qt = prof::now();
            let centre = 0.707 * (0.6 * s[2] + 2.0 * s[3]);
            let (mut l, mut r) = (s[0] + centre, s[1] + centre);
            if !(l.is_finite() && r.is_finite()) {
                l = 0.0;
                r = 0.0;
            }
            // level riding: halve the dynamic range in dB against a slowly adapting loud reference
            let e = 0.5 * (l * l + r * r);
            self.env += (e - self.env) * if e > self.env { att } else { rel };
            let rms = self.env.sqrt();
            if rms > self.level {
                self.level += (rms - self.level) * 0.0005;
            } else {
                self.level += (rms.max(1.0) - self.level) * 0.000002;
            }
            let g = (1.0 / (rms.max(0.01 * self.level) / self.level).sqrt()).min(20.0) * 0.28 / self.level;
            self.fade = (self.fade + 1.0 / 2400.0).min(1.0);
            fr[0] = ((l * g).tanh() * 0.95 * self.fade) as f32;
            fr[1] = ((r * g).tanh() * 0.95 * self.fade) as f32;
            prof::lap(cat::OUTPUT, qt);
        }
    }

    fn set_drive(&mut self, on: bool) {
        if on {
            if matches!(self.sys.mode, Mode::Drive(_)) {
                return;
            }
            // the engine's own vehicle, or the generic car (engine.rs, Drive::for_engine), from rest
            let mut d = Drive::for_engine(&self.spec, 0, 0.0);
            d.auto = true;
            d.locked = false;
            d.down_rpm = Drive::auto_down_rpm(&self.spec);
            self.sys.mode = Mode::Drive(d);
        } else {
            self.sys.mode = Mode::Free;
        }
    }
}

fn header_json(sys: &System, s: &EngineSpec, samples: &[usize], frame_floats: usize, quality: &str, rate: f64) -> serde_json::Value {
    let mut off = 8 + 6 * sys.cyls.len();
    let mut pipes = Vec::new();
    for (p, &sm) in sys.pipes.iter().zip(samples) {
        let dia: Vec<f64> = (0..sm).map(|kk| p.dia[((kk * p.n) / sm).min(p.n - 1)]).collect();
        let end = |e: crate::pipe::End| match e {
            crate::pipe::End::Closed => serde_json::json!({"type":"valve"}),
            crate::pipe::End::Ambient => serde_json::json!({"type":"ambient"}),
            crate::pipe::End::Volume(i) => serde_json::json!({"type":"volume","index":i,"name":sys.vols[i].name}),
        };
        pipes.push(serde_json::json!({"name":p.name,"kind":p.kind,"cyl":p.cyl,"group":p.group,"len":p.len,"samples":sm,"offset":off,"dia":dia,"left":end(p.left),"right":end(p.right)}));
        off += sm * 3;
    }
    let vols: Vec<_> = sys.vols.iter().enumerate().map(|(i, v)| serde_json::json!({"name":v.name,"vol":v.v,"offset":off + 2*i})).collect();
    let sound = sys.sound_header(off + 2 * sys.vols.len() + sys.collectors.len());
    let pins: Vec<f64> = (0..s.n()).map(|i| s.pin_angle(i)).collect();
    let dirs: Vec<f64> = (0..s.n()).map(|i| s.bank_dir(s.bank[i])).collect();
    let (af_off, af_n) = sys.afterfire_layout(samples);
    let mut h = serde_json::json!({
        "rate": FRAME_RATE, "t0": 0.0, "mode": "strobe", "strobe_deg": [9.0, 26.0], "live": true, "quality": quality,
        "frames": 0, "frame_floats": frame_floats,
        "globals": ["t","theta","rpm","throttle","torque","gear","speed","cut"],
        "cyl_fields": ["p","T","lift_in","lift_ex","burn","cycle_angle"], "cyl_offset": 8,
        "pipe_fields": ["p","T","u"], "vol_fields": ["p","T"],
        "p_atm": P_ATM, "spec": s, "kind": s.kind, "units": s.units(), "cycle_deg": s.cycle_deg(), "pin_angle": pins, "bank_dir": dirs,
        "pipes": pipes, "volumes": vols, "sound": sound, "audio_rate": 48000, "inner_rate": rate,
        "afterfire": {"offset": af_off, "count": af_n, "glow_ms": 25.0},
    });
    // supercharged engines only, so every other engine's header is unchanged
    if let Some(b) = sys.boost_header(samples) {
        h["boost"] = b;
    }
    if let Some(t) = sys.turbo_header(samples) {
        h["turbo"] = t;
    }
    // engines with a vehicle block only, so every other engine's header is unchanged
    if let Some(v) = vehicle_json(s) {
        h["vehicle"] = v;
    }
    h
}

/// What the app shows of the vehicle an engine pushes in Drive (docs/physics.md, "Vehicles in Drive"):
/// name, kind, mass with the driver or rider, gear count. None for the generic car.
pub fn vehicle_json(s: &EngineSpec) -> Option<serde_json::Value> {
    let v = s.vehicle.as_ref()?;
    let d = Drive::for_engine(s, 0, 0.0);
    Some(serde_json::json!({"name": v.name, "kind": v.kind, "mass_kg": d.mass, "gear_count": d.gears.len()}))
}

// ---------------------------------------------------------------- C ABI for WebAssembly

#[no_mangle]
pub extern "C" fn es_alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `p` must come from `es_alloc(n)`.
#[no_mangle]
pub unsafe extern "C" fn es_free(p: *mut u8, n: usize) {
    drop(Vec::from_raw_parts(p, 0, n.max(1)));
}

static mut PRESETS: Option<Vec<u8>> = None;

#[allow(static_mut_refs)]
fn presets() -> &'static Vec<u8> {
    unsafe {
        if PRESETS.is_none() {
            let list: Vec<_> = spec::names()
                .iter()
                .map(|n| {
                    let s = spec::preset(n).unwrap();
                    let mut e = serde_json::json!({"name": s.name, "label": s.label, "cylinders": s.n(), "layout": s.layout, "redline": s.redline, "idle": s.idle, "itb": s.itb, "litres": s.disp() * 1000.0, "kind": s.kind, "units": s.units(), "chamber_cc": s.disp_cyl() * 1.0e6, "spec": s});
                    // the library's induction label, only on engines that came boosted from the factory
                    if let Some(b) = &s.boost {
                        e["induction"] = serde_json::json!(b.induction);
                    }
                    // and the vehicle, only on engines whose definition has one
                    if let Some(v) = vehicle_json(&s) {
                        e["vehicle"] = v;
                    }
                    e
                })
                .collect();
            PRESETS = Some(serde_json::to_vec(&list).unwrap());
        }
        PRESETS.as_ref().unwrap()
    }
}

#[no_mangle]
pub extern "C" fn es_presets_ptr() -> *const u8 {
    presets().as_ptr()
}
#[no_mangle]
pub extern "C" fn es_presets_len() -> usize {
    presets().len()
}

/// Create a session from a JSON request {"engine": name, "overrides": {...}}. Returns null on a bad request.
/// # Safety
/// `json` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn es_new(json: *const u8, len: usize, quality: u32, start_rpm: f64) -> *mut Live {
    let bytes = std::slice::from_raw_parts(json, len);
    let req: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(_) => return std::ptr::null_mut(),
    };
    match Live::new(&req, quality as usize, start_rpm) {
        Some(l) => Box::into_raw(Box::new(l)),
        None => std::ptr::null_mut(),
    }
}

/// # Safety
/// All functions below take a pointer returned by `es_new` that has not been freed.
#[no_mangle]
pub unsafe extern "C" fn es_drop(l: *mut Live) {
    if !l.is_null() {
        drop(Box::from_raw(l));
    }
}
#[no_mangle]
pub unsafe extern "C" fn es_header_ptr(l: *const Live) -> *const u8 {
    (*l).header.as_ptr()
}
#[no_mangle]
pub unsafe extern "C" fn es_header_len(l: *const Live) -> usize {
    (*l).header.len()
}
#[no_mangle]
pub unsafe extern "C" fn es_process(l: *mut Live, out: *mut f32, frames: usize) {
    (*l).process(std::slice::from_raw_parts_mut(out, frames * 2));
}
#[no_mangle]
pub unsafe extern "C" fn es_frame_ptr(l: *const Live) -> *const f32 {
    (*l).frame.as_ptr()
}
#[no_mangle]
pub unsafe extern "C" fn es_frame_len(l: *const Live) -> usize {
    (*l).frame.len()
}
#[no_mangle]
pub unsafe extern "C" fn es_frame_seq(l: *const Live) -> u32 {
    (*l).frame_seq
}
#[no_mangle]
pub unsafe extern "C" fn es_rpm(l: *const Live) -> f64 {
    (*l).sys.rpm()
}
#[no_mangle]
pub unsafe extern "C" fn es_set_throttle(l: *mut Live, x: f64) {
    (*l).sys.throttle_cmd = x.clamp(0.0, 1.0);
}
#[no_mangle]
pub unsafe extern "C" fn es_set_drive(l: *mut Live, on: u32) {
    (*l).set_drive(on != 0);
}
#[no_mangle]
pub unsafe extern "C" fn es_set_auto(l: *mut Live, on: u32) {
    if let Mode::Drive(d) = &mut (*l).sys.mode {
        d.auto = on != 0;
    }
}
#[no_mangle]
pub unsafe extern "C" fn es_set_brake(l: *mut Live, x: f64) {
    if let Mode::Drive(d) = &mut (*l).sys.mode {
        d.brake = x.clamp(0.0, 1.0);
    }
}
#[no_mangle]
pub unsafe extern "C" fn es_shift(l: *mut Live, dir: i32) {
    if let Mode::Drive(d) = &mut (*l).sys.mode {
        let g = d.gear as i32 + dir.signum();
        if g >= 0 && (g as usize) < d.gears.len() && d.shifting <= 0.0 {
            d.gear = g as usize;
            d.shifting = d.shift_time;
        }
    }
}

/// Range of the live mixture control (equivalence ratio).
pub const PHI_MIN: f64 = 0.7;
pub const PHI_MAX: f64 = 1.7;

/// Fuel-air equivalence ratio, clamped to PHI_MIN..PHI_MAX (1.0 is stoichiometric).
#[no_mangle]
pub unsafe extern "C" fn es_set_phi(l: *mut Live, phi: f64) {
    if phi.is_finite() {
        (*l).sys.phi = phi.clamp(PHI_MIN, PHI_MAX);
    }
}
/// Share of overrun cycles that are fueled instead of cut, 0..1.
#[no_mangle]
pub unsafe extern "C" fn es_set_overrun_fuel(l: *mut Live, x: f64) {
    if x.is_finite() {
        (*l).sys.overrun_fuel = x.clamp(0.0, 1.0);
    }
}
/// Lift-off pops, 0..1: how much fuel wets the port walls and feeds the first cycles after
/// the throttle shuts (0 = none, the original model).
#[no_mangle]
pub unsafe extern "C" fn es_set_liftoff_pops(l: *mut Live, x: f64) {
    if x.is_finite() {
        (*l).sys.liftoff = x.clamp(0.0, 1.0);
    }
}
/// Turbo blow-off valve routing while running: 0 none (shut), 1 recirculating, 2 venting to the
/// air. Returns 1 if the engine has a turbo, 0 otherwise (nothing changes then).
#[no_mangle]
pub unsafe extern "C" fn es_set_blowoff(l: *mut Live, mode: u32) -> u32 {
    let m = match mode {
        0 => "none",
        1 => "recirc",
        _ => "vent",
    };
    (*l).sys.set_blowoff(m) as u32
}
/// Turbo whistle stand-in gain while running (0 = none, 1.0 is the default, clamped to 0..16).
#[no_mangle]
pub unsafe extern "C" fn es_set_whistle(l: *mut Live, x: f64) {
    if x.is_finite() {
        (*l).sys.set_whistle(x);
    }
}
/// Rev limiter: 0 cuts fuel, 1 cuts spark.
#[no_mangle]
pub unsafe extern "C" fn es_set_limiter(l: *mut Live, spark: u32) {
    (*l).sys.limiter_kind = if spark != 0 { Limiter::SparkCut } else { Limiter::FuelCut };
}
