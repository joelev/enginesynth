use engsim::engine::*;
use engsim::gas::*;
use engsim::{audio, pipe, quality, spec};
use serde::Deserialize;
use std::f64::consts::PI;
use std::io::Write;
use std::path::Path;

mod boostcheck;
mod liftcheck;
mod turbocheck;

#[derive(Deserialize)]
struct DriveCfg {
    mass: f64,
    wheel_r: f64,
    final_drive: f64,
    gears: Vec<f64>,
    shift_rpm: f64,
    shift_time: f64,
    #[serde(default)]
    start_gear: usize,
    #[serde(default = "default_cda")]
    cda: f64,
}
fn default_cda() -> f64 {
    0.7
}

#[derive(Deserialize)]
struct Scene {
    engine: String,
    #[serde(default)]
    mode: String,
    duration: f64,
    #[serde(default = "default_warmup")]
    warmup: f64,
    #[serde(default)]
    quality: String,
    start_rpm: Option<f64>,
    /// [time, throttle 0..1] keys, linear between
    throttle: Vec<[f64; 2]>,
    /// [time, rpm] keys for dyno mode
    #[serde(default)]
    rpm: Vec<[f64; 2]>,
    drive: Option<DriveCfg>,
    #[serde(default = "default_tele_rate")]
    tele_rate: f64,
    #[serde(default)]
    tele_t0: f64,
    tele_t1: Option<f64>,
    #[serde(default = "default_spm")]
    tele_spm: f64,
    /// "realtime" (a snapshot every 1/tele_rate s) or "strobe" (each frame steps the crank
    /// phase by a few degrees, taken from the most recent moment the engine was at that phase)
    #[serde(default)]
    tele_mode: String,
    /// strobe phase step per frame in crank degrees: [at zero rpm, at redline]
    #[serde(default = "default_strobe")]
    strobe_deg: [f64; 2],
    #[serde(default)]
    overrides: serde_json::Value,
    #[serde(default = "default_seed")]
    seed: u64,
    #[serde(default = "default_one")]
    mix_exhaust: f64,
    #[serde(default = "default_one")]
    mix_intake: f64,
    #[serde(default = "default_one")]
    mix_block: f64,
    /// 0 = no level compression, 0.5 = halve the dynamic range in dB
    #[serde(default = "default_compress")]
    compress: f64,
    /// fuel-air equivalence ratio (default 1.0)
    #[serde(default = "default_one")]
    phi: f64,
    /// share of overrun cycles that are fueled, 0..1 (default 0)
    #[serde(default)]
    overrun_fuel: f64,
    /// "fuel" (default) or "spark"
    #[serde(default)]
    limiter: String,
    /// lift-off pops, 0..1 (default 0)
    #[serde(default)]
    liftoff_pops: f64,
}
fn default_warmup() -> f64 {
    0.8
}
fn default_tele_rate() -> f64 {
    60.0
}
fn default_strobe() -> [f64; 2] {
    [9.0, 26.0]
}
fn default_spm() -> f64 {
    40.0
}
fn default_seed() -> u64 {
    0x9E3779B97F4A7C15
}
fn default_compress() -> f64 {
    0.5
}
fn default_one() -> f64 {
    1.0
}

fn interp(keys: &[[f64; 2]], t: f64) -> f64 {
    if keys.is_empty() {
        return 0.0;
    }
    if t <= keys[0][0] {
        return keys[0][1];
    }
    for w in keys.windows(2) {
        if t < w[1][0] {
            let f = (t - w[0][0]) / (w[1][0] - w[0][0]).max(1e-9);
            return w[0][1] + f * (w[1][1] - w[0][1]);
        }
    }
    keys[keys.len() - 1][1]
}

/// A built-in engine by id, or a definition file by path (anything ending in .json).
fn load_spec(name: &str) -> spec::EngineSpec {
    if name.ends_with(".json") {
        let text = std::fs::read_to_string(name).unwrap_or_else(|e| {
            eprintln!("cannot read {name}: {e}");
            std::process::exit(2)
        });
        return spec::parse(&text).unwrap_or_else(|e| {
            eprintln!("{name}: {e}");
            std::process::exit(2)
        });
    }
    spec::preset(name).unwrap_or_else(|| {
        eprintln!("unknown engine '{}'. Known: {:?}", name, spec::names());
        std::process::exit(2)
    })
}

/// Steady full-throttle running at one speed: (torque N.m, power kW, volumetric efficiency).
fn dyno_point(s: &spec::EngineSpec, q: &Quality, rpm: f64) -> (f64, f64, f64) {
    let mut sys = System::build(s, q, 12345);
    sys.mode = Mode::Dyno;
    sys.omega = rpm * PI / 30.0;
    sys.throttle_cmd = 1.0;
    let per_cycle = (120.0 / rpm * q.rate) as usize;
    // a turbo has to spool and its boost controller settle: 7 s first (other engines: as before)
    let warm = if sys.turbos.is_empty() { per_cycle * 10 } else { (7.0 * q.rate) as usize };
    for _ in 0..warm {
        sys.advance();
    }
    sys.reset_stats();
    for _ in 0..per_cycle * 6 {
        sys.advance();
    }
    let st = sys.stats;
    let torque = (st.work - st.fric_work - st.drive_work) / st.angle;
    (torque, torque * rpm * PI / 30.0 / 1000.0, st.air / (st.angle / (4.0 * PI)) / (RHO_ATM * s.disp()))
}

/// Compare each real engine with its published peak torque and power.
fn validate(ids: &[String], write: bool) {
    let q = quality("live96");
    let mut rows = Vec::new();
    for id in ids {
        let s = load_spec(id);
        let Some(p) = s.published.clone() else {
            if !s.generic {
                eprintln!("{}: no published figures in its definition, skipped", s.name);
            }
            continue;
        };
        if s.generic {
            continue;
        }
        let pct = |sim: f64, real: f64| if real > 0.0 { format!("{:+.0}%", 100.0 * (sim / real - 1.0)) } else { "".into() };
        let (mut tq, mut kw) = (String::from("n/a"), String::from("n/a"));
        if p.torque_nm > 0.0 && p.torque_rpm > 0.0 {
            let (t, _, _) = dyno_point(&s, &q, p.torque_rpm);
            tq = format!("{:.0} vs {:.0} N·m at {:.0} rpm ({})", t, p.torque_nm, p.torque_rpm, pct(t, p.torque_nm));
        }
        if p.power_kw > 0.0 && p.power_rpm > 0.0 {
            let (_, k, _) = dyno_point(&s, &q, p.power_rpm);
            kw = format!("{:.0} vs {:.0} kW at {:.0} rpm ({})", k, p.power_kw, p.power_rpm, pct(k, p.power_kw));
        }
        println!("{:<16} torque {}   power {}", s.name, tq, kw);
        rows.push(format!("| `{}` | {} | {} | {} |", s.name, s.based_on, tq, kw));
    }
    if rows.is_empty() {
        println!("no real engines with published figures to check");
    }
    if write {
        let mut md = String::from("# Model versus published output\n\nWritten by `engsim validate --write`. Simulated full-throttle torque and power at each engine's published peak-torque and peak-power speeds (quality `live96`), against the published figures in its definition. Generic engines are not listed.\n\n| Engine | Modelled on | Torque: simulated vs published | Power: simulated vs published |\n|---|---|---|---|\n");
        if rows.is_empty() {
            md.push_str("| (none yet) | | | |\n");
        }
        for r in &rows {
            md.push_str(r);
            md.push('\n');
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../engines/VALIDATION.md");
        std::fs::write(path, md).expect("cannot write engines/VALIDATION.md");
        println!("wrote engines/VALIDATION.md");
    }
}

fn spec_with_overrides(name: &str, ov: &serde_json::Value) -> spec::EngineSpec {
    let s = load_spec(name);
    if let Some(map) = ov.as_object() {
        let mut v = serde_json::to_value(&s).unwrap();
        for (k, val) in map {
            if v.get(k).is_none() {
                eprintln!("override '{}' is not a spec field", k);
                std::process::exit(2);
            }
            v[k] = val.clone();
        }
        return serde_json::from_value::<spec::EngineSpec>(v).expect("bad override value").finish().expect("override makes an invalid engine");
    }
    s
}

fn dyno(name: &str, qname: &str, rpms: &[f64], ov: &serde_json::Value) {
    let s = spec_with_overrides(name, ov);
    let q = quality(qname);
    println!("{} — {} ({:.2} L)", s.name, s.label, s.disp() * 1000.0);
    println!("{:>6} {:>8} {:>8} {:>7} {:>6} {:>8} {:>7} {:>7} {:>4}", "rpm", "Nm", "kW", "BMEP", "VE", "Ppeak", "Tcoll", "MAP", "sub");
    for &rpm in rpms {
        let mut sys = System::build(&s, &q, 12345);
        sys.mode = Mode::Dyno;
        sys.omega = rpm * PI / 30.0;
        sys.throttle_cmd = 1.0;
        let per_cycle = (120.0 / rpm * q.rate) as usize;
        let warm = if sys.turbos.is_empty() { per_cycle * 10 } else { (7.0 * q.rate) as usize };
        for _ in 0..warm {
            sys.advance();
        }
        sys.reset_stats();
        let cycles = 6;
        for _ in 0..per_cycle * cycles {
            sys.advance();
        }
        let st = sys.stats;
        let torque = (st.work - st.fric_work - st.drive_work) / st.angle;
        let kw = torque * rpm * PI / 30.0 / 1000.0;
        let bmep = torque * 4.0 * PI / s.disp() / 1e5;
        let ve = st.air / (st.angle / (4.0 * PI)) / (RHO_ATM * s.disp());
        println!("{:>6.0} {:>8.1} {:>8.1} {:>7.2} {:>6.3} {:>8.1} {:>7.0} {:>7.3} {:>4}   back-pressure: collector {:+.3} bar, last volume {:+.3} bar", rpm, torque, kw, bmep, ve, st.p_peak / 1e5, st.t_coll / st.n, st.p_man / st.n / 1e5, sys.max_sub, (st.p_coll / st.n - P_ATM) / 1e5, (st.p_last / st.n - P_ATM) / 1e5);
        if s.is_two_stroke() {
            let per = (st.angle / (2.0 * PI)) * RHO_ATM * s.disp_cyl() * s.n() as f64;
            println!("         two-stroke: into crankcase {:.2}, delivered to cylinder {:.2}, lost out the exhaust {:.2}, trapped {:.2} (per cycle, as a fraction of swept volume of air)", st.cc_in / per, st.delivered / per, st.spilled / per, st.air / per);
        }
    }
}

fn selftest() {
    use pipe::*;
    // 1. quarter-wave resonance of a closed-open tube
    let c = (GAMMA * R_GAS * T_ATM).sqrt();
    for &(len, d) in &[(1.0, 0.05), (0.5, 0.04)] {
        let mut p = Pipe::new("t", "t", &[Sec { len, d0: d, d1: d, fric: 0.0 }], 0.008, End::Closed, End::Ambient, T_ATM, T_ATM);
        for i in 0..p.n {
            let x = (i as f64 + 0.5) / p.n as f64;
            let dp = 200.0 * (-((x - 0.2) / 0.08).powi(2)).exp();
            let pr = P_ATM + dp;
            p.rho[i] = RHO_ATM * pow_1_g(pr / P_ATM);
            p.ene[i] = pr / GM1;
        }
        let dt = 1.0 / 192_000.0;
        let n = 192_000;
        let mut sig = Vec::with_capacity(n);
        for _ in 0..n {
            p.step(dt, Bc::Closed, Bc::Open);
            sig.push(p.prim(0).p - P_ATM);
        }
        // find the strongest frequency below 400 Hz with a direct DFT scan
        let mut best = (0.0, 0.0);
        let mut f = 20.0;
        while f < 400.0 {
            let (mut re, mut im) = (0.0, 0.0);
            for (k, s) in sig.iter().enumerate().step_by(8) {
                let ph = 2.0 * PI * f * k as f64 * dt;
                re += s * ph.cos();
                im += s * ph.sin();
            }
            let m = re * re + im * im;
            if m > best.1 {
                best = (f, m);
            }
            f += 0.1;
        }
        let ideal = c / (4.0 * len);
        let corrected = c / (4.0 * (len + 0.6133 * d / 2.0));
        let rmsw = |a: usize| (sig[a..a + 4800].iter().map(|s| s * s).sum::<f64>() / 4800.0).sqrt();
        let (e0, e1) = (rmsw(9600), rmsw(9600 + 38400));
        let tau = 0.2 / (e0 / e1).ln();
        println!("closed-open tube {len} m: measured {:.1} Hz, ideal c/4L {:.1} Hz, with end correction {:.1} Hz; ring-down Q = {:.0}", best.0, ideal, corrected, PI * best.0 * tau);
    }
    // 1b. the same tube solved along characteristics (wave.rs): a small pressure step sent in from the open end
    for &(len, d) in &[(1.0, 0.05), (0.5, 0.04)] {
        let mut p = Pipe::new("t", "t", &[Sec { len, d0: d, d1: d, fric: 0.0 }], 0.008, End::Closed, End::Ambient, T_ATM, T_ATM);
        p.make_wave();
        let dt = 1.0 / 192_000.0;
        let n = 192_000;
        let mut sig = Vec::with_capacity(n);
        for i in 0..n {
            // kick: hold the open end against a slightly raised reservoir for 1 ms, then open air
            let br = if i < 192 { Bc::Res { p0: P_ATM + 200.0, rho0: RHO_ATM } } else { Bc::Open };
            p.step(dt, Bc::Closed, br);
            sig.push(p.prim(0).p - P_ATM);
        }
        let mut best = (0.0, 0.0);
        let mut f = 20.0;
        while f < 400.0 {
            let (mut re, mut im) = (0.0, 0.0);
            for (k, s) in sig.iter().enumerate().skip(2000).step_by(8) {
                let ph = 2.0 * PI * f * k as f64 * dt;
                re += s * ph.cos();
                im += s * ph.sin();
            }
            let m = re * re + im * im;
            if m > best.1 {
                best = (f, m);
            }
            f += 0.1;
        }
        let rmsw = |a: usize| (sig[a..a + 4800].iter().map(|s| s * s).sum::<f64>() / 4800.0).sqrt();
        let (e0, e1) = (rmsw(9600), rmsw(9600 + 38400));
        let tau = 0.2 / (e0 / e1).ln();
        println!("  along characteristics, {len} m: measured {:.1} Hz, with end correction {:.1} Hz; ring-down Q = {:.0}", best.0, c / (4.0 * (len + 0.6133 * d / 2.0)), PI * best.0 * tau);
    }
    // 2. conservation in a sealed tube with a strong shock
    let mut p = Pipe::new("t", "t", &[Sec { len: 1.0, d0: 0.05, d1: 0.05, fric: 0.0 }], 0.008, End::Closed, End::Closed, T_ATM, T_ATM);
    p.t_wall = T_ATM;
    for i in 0..p.n / 2 {
        p.rho[i] *= 5.0;
        p.ene[i] *= 5.0;
    }
    let m0 = p.mass();
    let mut ok = true;
    for _ in 0..50_000 {
        p.step(1.0 / 192_000.0, Bc::Closed, Bc::Closed);
        if !(0..p.n).all(|i| p.rho[i].is_finite() && p.prim(i).p > 0.0) {
            ok = false;
            break;
        }
    }
    println!("sealed shock tube (5:1): mass drift {:.2e}, stayed finite and positive: {}", (p.mass() - m0) / m0, ok);
}

fn render(scene_path: &str, out_dir: &str) {
    let text = std::fs::read_to_string(scene_path).expect("cannot read scene");
    let sc: Scene = serde_json::from_str(&text).expect("bad scene JSON");
    let s = spec_with_overrides(&sc.engine, &sc.overrides);
    let q = quality(&sc.quality);
    let k = (q.rate / 48_000.0).round() as usize;
    let out = Path::new(out_dir);
    std::fs::create_dir_all(out).unwrap();
    let mut sys = System::build(&s, &q, sc.seed);
    sys.phi = sc.phi.clamp(0.5, 2.0);
    sys.overrun_fuel = sc.overrun_fuel.clamp(0.0, 1.0);
    sys.liftoff = sc.liftoff_pops.clamp(0.0, 1.0);
    sys.limiter_kind = match sc.limiter.as_str() {
        "" | "fuel" => Limiter::FuelCut,
        "spark" => Limiter::SparkCut,
        other => panic!("limiter must be \"fuel\" or \"spark\", not {other:?}"),
    };

    let start_rpm = sc.start_rpm.unwrap_or(if sc.rpm.is_empty() { s.idle } else { sc.rpm[0][1] });
    // warm-up: hold speed so the pipes fill with hot gas and the cycle settles
    sys.mode = Mode::Dyno;
    sys.omega = start_rpm * PI / 30.0;
    sys.throttle_cmd = interp(&sc.throttle, 0.0).max(0.06);
    for _ in 0..(sc.warmup * q.rate) as usize {
        sys.advance();
    }
    sys.mode = match sc.mode.as_str() {
        "dyno" => Mode::Dyno,
        "drive" => {
            let d = sc.drive.as_ref().expect("drive mode needs a 'drive' block");
            let g = d.gears[d.start_gear] * d.final_drive;
            Mode::Drive(Drive { mass: d.mass, wheel_r: d.wheel_r, final_drive: d.final_drive, gears: d.gears.clone(), shift_rpm: d.shift_rpm, shift_time: d.shift_time, gear: d.start_gear, v: sys.omega * d.wheel_r / g, shifting: 0.0, cda: d.cda, auto: true, down_rpm: 0.0, brake: 0.0, locked: true, clutch_nm_per_l: 130.0, clutch_gain: 1.5, clutch_launch: 0.0, eta: 1.0, grip: f64::INFINITY, t_avg: 0.0, grip_hl: 0.0, mu: 0.0, tc: 1.0 })
        }
        _ => Mode::Free,
    };
    sys.t = 0.0;

    // telemetry layout
    let samples: Vec<usize> = sys.pipes.iter().map(|p| ((p.len * sc.tele_spm).round() as usize).clamp(4, 400).min(p.n)).collect();
    let tele_t1 = sc.tele_t1.unwrap_or(sc.duration);
    let mut tele: Vec<f32> = Vec::new();
    let mut frames = 0usize;
    let mut next_frame = sc.tele_t0;
    let mut frame_floats = 0usize;

    struct Pending {
        frame: usize,
        phase: f64,
        data: Vec<f32>,
    }
    let strobe = sc.tele_mode == "strobe";
    let four_pi = s.cycle_deg() * PI / 180.0; // one chamber cycle: the strobe repeats over this
    let mut pending: std::collections::VecDeque<Pending> = std::collections::VecDeque::new();
    let mut created = 0usize;
    let mut last_phase = 0.0f64;
    let mut prev_theta = sys.theta;
    let (af_off, af_n) = sys.afterfire_layout(&samples);
    let (bo_off, bo_n) = sys.boost_layout(&samples);
    let bo_n = bo_n * System::BOOST_FIELDS.len() + sys.turbos.len() * System::TURBO_FIELDS.len();

    let mut dec = [audio::Decim::new(k), audio::Decim::new(k), audio::Decim::new(k), audio::Decim::new(k)];
    let n_inner = (sc.duration * q.rate) as usize;
    let mut stems: [Vec<f32>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let mut log = String::from("t,rpm,torque,throttle,gear,speed_kmh\n");
    let wall = std::time::Instant::now();
    let mut tq_acc = 0.0;
    for i in 0..n_inner {
        let t = i as f64 / q.rate;
        sys.throttle_cmd = interp(&sc.throttle, t);
        if matches!(sys.mode, Mode::Dyno) && !sc.rpm.is_empty() {
            sys.omega = interp(&sc.rpm, t) * PI / 30.0;
        }
        let o = sys.advance();
        tq_acc += sys.torque;
        for c in 0..4 {
            if let Some(v) = dec[c].push(o[c]) {
                stems[c].push(v as f32);
            }
        }
        if strobe {
            // create frames far enough ahead that each target phase comes round before it is shown
            let rpm = sys.rpm().max(60.0);
            let look = ((sc.tele_rate * 120.0 / rpm).ceil() as usize + 1).clamp(2, 14);
            let now_frame = ((t - sc.tele_t0) * sc.tele_rate).floor().max(0.0) as usize;
            while t >= sc.tele_t0 - look as f64 / sc.tele_rate && created <= now_frame + look && (sc.tele_t0 + created as f64 / sc.tele_rate) <= tele_t1 {
                let step = (sc.strobe_deg[0] + (sc.strobe_deg[1] - sc.strobe_deg[0]) * (rpm / s.redline).min(1.0)).min(rpm * 6.0 / sc.tele_rate);
                last_phase = (last_phase + step * PI / 180.0).rem_euclid(four_pi);
                let mut data = Vec::new();
                sys.snapshot(&samples, &mut data);
                pending.push_back(Pending { frame: created, phase: last_phase, data });
                created += 1;
            }
            for pnd in pending.iter_mut() {
                let a = (prev_theta - pnd.phase).rem_euclid(four_pi);
                let b = (sys.theta - pnd.phase).rem_euclid(four_pi);
                if b < a {
                    pnd.data.clear();
                    sys.snapshot(&samples, &mut pnd.data);
                }
            }
            while let Some(front) = pending.front() {
                if t < sc.tele_t0 + front.frame as f64 / sc.tele_rate {
                    break;
                }
                let mut pnd = pending.pop_front().unwrap();
                let mut cur = Vec::new();
                sys.snapshot(&samples, &mut cur);
                for gi in [0usize, 2, 3, 4, 5, 6, 7].into_iter().chain(af_off..af_off + af_n).chain(bo_off..bo_off + bo_n) {
                    pnd.data[gi] = cur[gi];
                }
                frame_floats = pnd.data.len();
                tele.extend_from_slice(&pnd.data);
                frames += 1;
            }
            prev_theta = sys.theta;
        } else if t >= next_frame && t <= tele_t1 {
            let before = tele.len();
            sys.snapshot(&samples, &mut tele);
            frame_floats = tele.len() - before;
            frames += 1;
            next_frame += 1.0 / sc.tele_rate;
        }
        if i % (q.rate as usize / 50) == 0 {
            let (gear, v) = match &sys.mode {
                Mode::Drive(d) => (d.gear + 1, d.v * 3.6),
                _ => (0, 0.0),
            };
            log.push_str(&format!("{:.3},{:.0},{:.1},{:.3},{},{:.1}\n", t, sys.rpm(), tq_acc / (q.rate / 50.0), sys.throttle_eff, gear, v));
            tq_acc = 0.0;
        }
    }
    let secs = wall.elapsed().as_secs_f64();

    // stems (pascals at 1 m) and the mix
    let n = stems[0].len();
    let rms = |v: &[f32]| (v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / v.len().max(1) as f64).sqrt();
    let mut ex = Vec::with_capacity(n * 2);
    for i in 0..n {
        ex.push(stems[0][i]);
        ex.push(stems[1][i]);
    }
    audio::write_wav_f32(&out.join("stem_exhaust.wav"), 2, 48_000, &ex).unwrap();
    audio::write_wav_f32(&out.join("stem_intake.wav"), 1, 48_000, &stems[2]).unwrap();
    audio::write_wav_f32(&out.join("stem_block.wav"), 1, 48_000, &stems[3]).unwrap();
    let mut mix = Vec::with_capacity(n * 2);
    for i in 0..n {
        let c = sc.mix_intake as f32 * stems[2][i] + sc.mix_block as f32 * stems[3][i];
        mix.push(sc.mix_exhaust as f32 * stems[0][i] + 0.707 * c);
        mix.push(sc.mix_exhaust as f32 * stems[1][i] + 0.707 * c);
    }
    // gentle level riding so idle stays audible next to full throttle (RMS follower, slow release)
    if sc.compress > 0.0 {
        let (att, rel) = (1.0 - (-1.0f64 / (0.010 * 48_000.0)).exp(), 1.0 - (-1.0f64 / (0.250 * 48_000.0)).exp());
        let mut env = 0.0f64;
        let mut envs = Vec::with_capacity(n);
        for i in 0..n {
            let e = 0.5 * ((mix[2 * i] as f64).powi(2) + (mix[2 * i + 1] as f64).powi(2));
            env += (e - env) * if e > env { att } else { rel };
            envs.push(env.sqrt());
        }
        let mut sorted = envs.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let top = sorted[(sorted.len() as f64 * 0.98) as usize].max(1e-9);
        for i in 0..n {
            let gcomp = (envs[i].max(top * 0.01) / top).powf(-sc.compress).min(20.0) as f32;
            mix[2 * i] *= gcomp;
            mix[2 * i + 1] *= gcomp;
        }
    }
    // level: scale so the 99.9th percentile sits at 0.7, then a soft knee
    let mut mags: Vec<f32> = mix.iter().map(|x| x.abs()).collect();
    mags.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let refl = mags[((mags.len() as f64 * 0.999) as usize).min(mags.len() - 1)].max(1e-9);
    let g = 0.7 / refl;
    for x in mix.iter_mut() {
        *x = (*x * g).tanh() * 0.97;
    }
    audio::write_wav16(&out.join("mix.wav"), 2, 48_000, &mix).unwrap();
    std::fs::write(out.join("log.csv"), log).unwrap();

    // telemetry
    let mut f = std::io::BufWriter::new(std::fs::File::create(out.join("telemetry.bin")).unwrap());
    for v in &tele {
        f.write_all(&v.to_le_bytes()).unwrap();
    }
    let mut off = 8 + 6 * sys.cyls.len();
    let mut pipes_json = Vec::new();
    for (p, &sm) in sys.pipes.iter().zip(&samples) {
        let dia: Vec<f64> = (0..sm).map(|kk| p.dia[((kk * p.n) / sm).min(p.n - 1)]).collect();
        let end = |e: pipe::End| match e {
            pipe::End::Closed => serde_json::json!({"type":"valve"}),
            pipe::End::Ambient => serde_json::json!({"type":"ambient"}),
            pipe::End::Volume(i) => serde_json::json!({"type":"volume","index":i,"name":sys.vols[i].name}),
        };
        pipes_json.push(serde_json::json!({"name":p.name,"kind":p.kind,"cyl":p.cyl,"group":p.group,"len":p.len,"samples":sm,"offset":off,"dia":dia,"left":end(p.left),"right":end(p.right)}));
        off += sm * 3;
    }
    let vols_json: Vec<_> = sys.vols.iter().enumerate().map(|(i, v)| serde_json::json!({"name":v.name,"vol":v.v,"offset":off + 2*i})).collect();
    let sound = sys.sound_header(off + 2 * sys.vols.len() + sys.collectors.len());
    let pins: Vec<f64> = (0..s.n()).map(|i| s.pin_angle(i)).collect();
    let dirs: Vec<f64> = (0..s.n()).map(|i| s.bank_dir(s.bank[i])).collect();
    let mut header = serde_json::json!({
        "rate": sc.tele_rate, "t0": sc.tele_t0, "mode": if strobe { "strobe" } else { "realtime" }, "strobe_deg": sc.strobe_deg, "frames": frames, "frame_floats": frame_floats,
        "globals": ["t","theta","rpm","throttle","torque","gear","speed","cut"],
        "cyl_fields": ["p","T","lift_in","lift_ex","burn","cycle_angle"], "cyl_offset": 8,
        "pipe_fields": ["p","T","u"], "vol_fields": ["p","T"],
        "p_atm": P_ATM, "spec": s, "kind": s.kind, "units": s.units(), "cycle_deg": s.cycle_deg(), "pin_angle": pins, "bank_dir": dirs,
        "pipes": pipes_json, "volumes": vols_json, "sound": sound, "duration": sc.duration, "audio_rate": 48000, "inner_rate": q.rate,
        "afterfire": {"offset": af_off, "count": af_n, "glow_ms": 25.0}, "phi": sys.phi, "overrun_fuel": sys.overrun_fuel, "liftoff_pops": sys.liftoff, "limiter": if sys.limiter_kind == Limiter::SparkCut { "spark" } else { "fuel" },
    });
    if let Some(b) = sys.boost_header(&samples) {
        header["boost"] = b;
    }
    if let Some(t) = sys.turbo_header(&samples) {
        header["turbo"] = t;
    }
    std::fs::write(out.join("telemetry.json"), serde_json::to_string(&header).unwrap()).unwrap();

    let db = |p: f64| 20.0 * (p / 2e-5).max(1e-12).log10();
    println!("{}: {:.1} s of audio in {:.1} s ({:.1}x real time), {} telemetry frames, max substeps {}", s.name, sc.duration, secs, secs / sc.duration, frames, sys.max_sub);
    println!("  stem level at 1 m: exhaust {:.0} dB SPL, intake {:.0} dB, block {:.0} dB; mix gain {:.3}", db(rms(&ex)), db(rms(&stems[2])), db(rms(&stems[3])), g);
}

/// Peak level over 10 ms windows (5 ms hop) of the exhaust microphone, dB SPL.
fn peak_level(e: &[f64], rate: f64) -> f64 {
    let w = (0.010 * rate) as usize;
    let hop = (w / 2).max(1);
    let mut best = 0.0f64;
    let mut i = 0;
    while i + w <= e.len() {
        best = best.max(e[i..i + w].iter().sum::<f64>() / w as f64);
        i += hop;
    }
    20.0 * (best.sqrt() / 2e-5).max(1e-12).log10()
}

/// Level of the 10 ms window around each afterfire onset (1 ms before to 9 ms after), dB SPL:
/// (median, 90th percentile, max) over the events, or None without events.
fn event_levels(e: &[f64], events: &[f64], rate: f64) -> Option<(f64, f64, f64)> {
    let w = (0.010 * rate) as usize;
    let mut l: Vec<f64> = events
        .iter()
        .filter_map(|&t| {
            let i0 = ((t - 0.001) * rate).max(0.0) as usize;
            (i0 + w <= e.len()).then(|| 20.0 * ((e[i0..i0 + w].iter().sum::<f64>() / w as f64).sqrt() / 2e-5).max(1e-12).log10())
        })
        .collect();
    if l.is_empty() {
        return None;
    }
    l.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Some((l[l.len() / 2], l[(l.len() * 9 / 10).min(l.len() - 1)], l[l.len() - 1]))
}

fn mean_level(e: &[f64]) -> f64 {
    20.0 * ((e.iter().sum::<f64>() / e.len().max(1) as f64).sqrt() / 2e-5).max(1e-12).log10()
}

struct AfRun {
    /// exhaust microphone power 0.5 (L^2 + R^2) per inner sample
    e: Vec<f64>,
    /// afterfire onsets, seconds from the start of the run
    events: Vec<f64>,
    /// rpm every 100 inner samples
    rpm: Vec<f64>,
    finite: bool,
}

fn af_count(sys: &System) -> u64 {
    sys.af.iter().map(|a| a.events).sum()
}

/// Run `secs`, letting `ctl(sys, t)` set the controls before each step.
fn af_run(sys: &mut System, secs: f64, mut ctl: impl FnMut(&mut System, f64)) -> AfRun {
    let rate = 1.0 / sys.dt;
    let n = (secs * rate) as usize;
    let mut r = AfRun { e: Vec::with_capacity(n), events: Vec::new(), rpm: Vec::with_capacity(n / 100 + 1), finite: true };
    let mut c0 = af_count(sys);
    for i in 0..n {
        let t = i as f64 / rate;
        ctl(sys, t);
        let o = sys.advance();
        r.finite &= o.iter().all(|v| v.is_finite()) && sys.omega.is_finite();
        r.e.push(0.5 * (o[0] * o[0] + o[1] * o[1]));
        let c = af_count(sys);
        if c > c0 {
            for _ in c0..c {
                r.events.push(t);
            }
            c0 = c;
        }
        if i % 100 == 0 {
            r.rpm.push(sys.rpm());
        }
    }
    r
}

fn af_torque(s: &spec::EngineSpec, q: &Quality, rpm: f64, phi: f64) -> f64 {
    let mut sys = System::build(s, q, 12345);
    sys.phi = phi;
    sys.mode = Mode::Dyno;
    sys.omega = rpm * PI / 30.0;
    sys.throttle_cmd = 1.0;
    let per_cycle = (s.cycle_deg() / 360.0 * 60.0 / rpm * q.rate) as usize;
    for _ in 0..per_cycle * 10 {
        sys.advance();
    }
    sys.reset_stats();
    for _ in 0..per_cycle * 16 {
        sys.advance();
    }
    let st = sys.stats;
    (st.work - st.fric_work - st.drive_work) / st.angle
}

/// The live app's car for an engine (its own vehicle, or the generic car), in `gear` (0-based) at engine speed `omega`.
fn generic_car(s: &spec::EngineSpec, gear: usize, omega: f64) -> Drive {
    Drive::for_engine(s, gear, omega)
}

/// Checks for the mixture and afterfire model: `engsim afterfire [a b c d e f] [--quality q]`.
fn afterfire_checks(qname: &str, parts: &[String]) {
    let q = quality(qname);
    let want = |p: &str| parts.is_empty() || parts.iter().any(|x| x == p);
    println!("afterfire checks at quality {qname}");
    let mut fails = 0;
    let mut verdict = |name: &str, ok: bool| {
        println!("  {} {}", if ok { "PASS" } else { "FAIL" }, name);
        if !ok {
            fails += 1;
        }
    };
    let pair = ["v8_cross", "i4"];
    let lim_name = |k: Limiter| if k == Limiter::SparkCut { "spark cut" } else { "fuel cut" };
    if want("a") {
        println!("(a) full-throttle torque (N·m) against equivalence ratio, dyno at 4000 rpm, 16 cycles");
        let phis = [0.62, 0.65, 0.7, 0.8, 0.9, 1.0, 1.05, 1.1, 1.15, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7];
        for id in pair {
            let s = load_spec(id);
            let tq: Vec<f64> = phis.iter().map(|&p| af_torque(&s, &q, 4000.0, p)).collect();
            let (ib, best) = tq.iter().enumerate().fold((0, f64::MIN), |b, (i, &t)| if t > b.1 { (i, t) } else { b });
            println!("  {:<9} {}", id, phis.iter().zip(&tq).map(|(p, t)| format!("{p:.2}:{t:.0}")).collect::<Vec<_>>().join(" "));
            let at = |p: f64| tq[phis.iter().position(|&x| (x - p).abs() < 1e-9).unwrap()];
            let rel = |p: f64| 100.0 * (at(p) / at(1.0) - 1.0);
            println!("            peak {:.0} at phi {:.2} ({:+.1}% on phi 1.0); {:+.1}% at 0.8, {:+.1}% at 0.7, {:+.1}% at 0.62, {:+.1}% at 1.3, {:+.1}% at 1.5, {:+.1}% at 1.7", best, phis[ib], 100.0 * (best / at(1.0) - 1.0), rel(0.8), rel(0.7), rel(0.62), rel(1.3), rel(1.5), rel(1.7));
            verdict(&format!("{id}: peak slightly rich (1.02..1.2), falling both ways"), phis[ib] > 1.0 && phis[ib] <= 1.2 && at(0.8) < best && at(0.7) < at(0.8) && at(1.5) < best && at(1.7) < at(1.5));
        }
    }
    // full-throttle reference level for (e): dyno, phi 1.0, at 80% of redline
    let mut ft_ref = std::collections::HashMap::new();
    let mut lim_ref = std::collections::HashMap::new();
    let mut bang = std::collections::HashMap::<&str, Vec<(String, Option<(f64, f64, f64)>)>>::new();
    if want("e") {
        for id in pair {
            let s = load_spec(id);
            let mut sys = System::build(&s, &q, 21);
            sys.mode = Mode::Dyno;
            sys.omega = 0.8 * s.redline * PI / 30.0;
            sys.throttle_cmd = 1.0;
            af_run(&mut sys, 0.4, |_, _| {});
            let r = af_run(&mut sys, 1.0, |_, _| {});
            ft_ref.insert(id, (peak_level(&r.e, q.rate), mean_level(&r.e)));
        }
    }
    if want("b") || want("e") {
        println!("(b) rich mixture, then a throttle lift: full throttle on the dyno at 75% of redline for 1 s (steady), then free with the throttle shut for 1.5 s");
        for id in pair {
            let s = load_spec(id);
            for phi in [1.0, 1.15, 1.3] {
                let mut sys = System::build(&s, &q, 31);
                sys.phi = phi;
                sys.mode = Mode::Dyno;
                sys.omega = 0.75 * s.redline * PI / 30.0;
                sys.throttle_cmd = 1.0;
                af_run(&mut sys, 0.5, |_, _| {});
                let steady = af_run(&mut sys, 1.0, |_, _| {});
                sys.mode = Mode::Free;
                let lift = af_run(&mut sys, 1.5, |sy, _| sy.throttle_cmd = 0.0);
                let first: Vec<String> = lift.events.iter().take(8).map(|t| format!("{:.0}", t * 1000.0)).collect();
                let early = lift.events.iter().filter(|&&t| t < 0.5).count();
                println!("  {:<9} phi {:.2}: steady {} events; after the lift {} ({} in the first 0.5 s), first at ms [{}]; peak level after the lift {:.1} dB", id, phi, steady.events.len(), lift.events.len(), early, first.join(","), peak_level(&lift.e, q.rate));
                if phi == 1.3 {
                    verdict(&format!("{id}: rich lift pops at the start of the overrun, none steady"), steady.events.is_empty() && early > 0);
                    bang.entry(id).or_default().push(("rich lift (phi 1.3)".into(), event_levels(&lift.e, &lift.events, q.rate)));
                }
                if phi == 1.0 {
                    verdict(&format!("{id}: stoichiometric lift with the fuel cut stays quiet"), lift.events.is_empty() && steady.events.is_empty());
                }
            }
        }
    }
    if want("c") || want("e") {
        println!("(c) overrun fueling 1.0 through a long overrun: a car coasting in third gear from 85% of redline with the throttle shut for 4 s (the live app's generic car); and a free coast from 95% of redline");
        for id in pair {
            let s = load_spec(id);
            let mut sys = System::build(&s, &q, 41);
            sys.overrun_fuel = 1.0;
            sys.mode = Mode::Dyno;
            sys.omega = 0.85 * s.redline * PI / 30.0;
            sys.throttle_cmd = 0.5;
            af_run(&mut sys, 0.4, |_, _| {});
            sys.mode = Mode::Drive(generic_car(&s, 2, sys.omega));
            let charge_q = s.heat_q * RHO_ATM * s.disp_cyl();
            let q0: f64 = sys.af.iter().map(|a| a.q_sum).sum();
            let r = af_run(&mut sys, 4.0, |sy, _| sy.throttle_cmd = 0.0);
            let n_ev = r.events.len().max(1) as f64;
            let q_ev = (sys.af.iter().map(|a| a.q_sum).sum::<f64>() - q0) / n_ev;
            let iv: Vec<f64> = r.events.windows(2).map(|w| w[1] - w[0]).collect();
            let m = iv.iter().sum::<f64>() / iv.len().max(1) as f64;
            let sd = (iv.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / iv.len().max(1) as f64).sqrt();
            let late = r.events.iter().filter(|&&t| t > 1.0).count();
            let cv = sd / m.max(1e-12);
            println!("  {:<9} coasting in gear: rpm {:.0} -> {:.0}; {} events in 4 s ({} after the first second), interval mean {:.1} ms, sd {:.1} ms (CV {:.2}); heat per event {:.2} of a full cylinder charge's; peak level {:.1} dB", id, r.rpm[0], r.rpm[r.rpm.len() - 1], r.events.len(), late, 1000.0 * m, 1000.0 * sd, cv, q_ev / charge_q, peak_level(&r.e, q.rate));
            verdict(&format!("{id}: repeated, irregular afterfire through a long overrun"), late >= 10 && cv > 0.15);
            bang.entry(id).or_default().push(("coast in gear, fueled".into(), event_levels(&r.e, &r.events, q.rate)));
            for of in [0.0, 0.3, 1.0] {
                let mut sys = System::build(&s, &q, 43);
                sys.overrun_fuel = of;
                sys.mode = Mode::Dyno;
                sys.omega = 0.95 * s.redline * PI / 30.0;
                sys.throttle_cmd = 1.0;
                af_run(&mut sys, 0.4, |_, _| {});
                sys.mode = Mode::Free;
                let r = af_run(&mut sys, 3.0, |sy, _| sy.throttle_cmd = 0.0);
                println!("  {:<9} free coast, overrun fueling {:.1}: {} events; rpm {:.0} after 0.5 s, {:.0} after 1 s, {:.0} after 3 s", id, of, r.events.len(), r.rpm[r.rpm.len() / 6], r.rpm[r.rpm.len() / 3], r.rpm[r.rpm.len() - 1]);
            }
        }
    }
    if want("d") || want("e") {
        println!("(d) bouncing on the limiter at full throttle (free, 3 s from idle, phi 1.0)");
        for id in pair {
            let s = load_spec(id);
            for kind in [Limiter::FuelCut, Limiter::SparkCut] {
                let mut sys = System::build(&s, &q, 51);
                sys.limiter_kind = kind;
                sys.mode = Mode::Free;
                let mut lim_steps = 0usize;
                let mut lim_t: Option<f64> = None;
                let r = af_run(&mut sys, 3.0, |sy, t| {
                    sy.throttle_cmd = if t > 0.3 { 1.0 } else { 0.0 };
                    if sy.limiter {
                        lim_steps += 1;
                        lim_t.get_or_insert(t);
                    }
                });
                let t0 = lim_t.unwrap_or(3.0);
                let after = r.events.iter().filter(|&&t| t >= t0).count();
                let lvl = peak_level(&r.e[((t0 * q.rate) as usize).min(r.e.len())..], q.rate);
                let charge_q = s.heat_q * RHO_ATM * s.disp_cyl();
                let n_ev = sys.af.iter().map(|a| a.events).sum::<u64>().max(1) as f64;
                println!("  {:<9} {:<9}: on the limiter from {:.2} s ({:.0}% of the time after that); {} afterfire events there, heat per event mean {:.2}, largest {:.2} of a full cylinder charge's; loudest 10 ms there {:.1} dB", id, lim_name(kind), t0, 100.0 * lim_steps as f64 / ((3.0 - t0) * q.rate).max(1.0), after, sys.af.iter().map(|a| a.q_sum).sum::<f64>() / n_ev / charge_q, sys.af.iter().map(|a| a.q_max).fold(0.0, f64::max) / charge_q, lvl);
                if kind == Limiter::SparkCut {
                    verdict(&format!("{id}: spark-cut limiter pops"), after >= 5);
                    bang.entry(id).or_default().push(("spark-cut limiter".into(), event_levels(&r.e, &r.events, q.rate)));
                } else {
                    verdict(&format!("{id}: fuel-cut limiter does not"), after == 0);
                    lim_ref.insert(id, lvl);
                }
            }
        }
    }
    if want("e") {
        println!("(e) level of the 10 ms window at each bang, at the exhaust microphone, against the same engine's loudest 10 ms at steady full throttle (dyno, 80% of redline, phi 1.0)");
        for id in pair {
            let (pk, mn) = ft_ref[id];
            println!("  {:<9} full throttle: loudest 10 ms {:.1} dB, mean {:.1} dB; on the fuel-cut limiter (no afterfire) loudest 10 ms {:.1} dB", id, pk, mn, lim_ref.get(id).copied().unwrap_or(f64::NAN));
            for (what, l) in bang.get(id).cloned().unwrap_or_default() {
                match l {
                    Some((med, p90, mx)) => println!("            {:<22} bangs: median {:.1} dB ({:+.1}), 90th percentile {:.1} dB ({:+.1}), loudest {:.1} dB ({:+.1})", what, med, med - pk, p90, p90 - pk, mx, mx - pk),
                    None => println!("            {:<22} no bangs", what),
                }
            }
        }
    }
    if want("f") {
        let fq_name = std::env::var("ENGSIM_AF_FQ").unwrap_or_else(|_| "live48+W".into());
        println!("(f) every engine at the defaults and at every corner of the settings (phi at the live app's limits, overrun fueling 0 / 1, fuel / spark cut), quality {fq_name}: 0.6 s idle, full throttle to 2.2 s (into the limiter), lift, settle to 5 s; a stall is the lowest speed in the last second under half of idle");
        let names: Vec<&str> = spec::names().to_vec();
        let lo: f64 = std::env::var("ENGSIM_AF_PHI_LO").ok().and_then(|x| x.parse().ok()).unwrap_or(engsim::live::PHI_MIN);
        let hi: f64 = std::env::var("ENGSIM_AF_PHI_HI").ok().and_then(|x| x.parse().ok()).unwrap_or(engsim::live::PHI_MAX);
        // the defaults first, for reference: a failure there is not this model's
        let combos: Vec<(f64, f64, Limiter)> = std::iter::once((1.0, 0.0, Limiter::FuelCut)).chain([lo, hi].into_iter().flat_map(|p| [0.0, 1.0].iter().flat_map(move |&o| [Limiter::FuelCut, Limiter::SparkCut].into_iter().map(move |k| (p, o, k))))).collect();
        let fq = quality(&fq_name);
        let results = std::sync::Mutex::new(Vec::new());
        let next = std::sync::atomic::AtomicUsize::new(0);
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(10);
        std::thread::scope(|sc| {
            for _ in 0..threads {
                sc.spawn(|| loop {
                    let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if k >= names.len() * combos.len() {
                        break;
                    }
                    let (id, (phi, of, kind)) = (names[k / combos.len()], combos[k % combos.len()]);
                    let s = load_spec(id);
                    let mut sys = System::build(&s, &fq, 61);
                    sys.phi = phi;
                    sys.overrun_fuel = of;
                    sys.limiter_kind = kind;
                    sys.mode = Mode::Free;
                    let r = af_run(&mut sys, 5.0, |sy, t| sy.throttle_cmd = if (0.6..2.2).contains(&t) { 1.0 } else { 0.0 });
                    let per = r.rpm.len() as f64 / 5.0;
                    let settle = &r.rpm[(4.0 * per) as usize..];
                    let min_settle = settle.iter().cloned().fold(f64::MAX, f64::min);
                    let peak = r.rpm.iter().cloned().fold(0.0, f64::max);
                    results.lock().unwrap().push((k, id, phi, of, kind, r.finite, min_settle, s.idle, peak, s.redline, r.events.len()));
                });
            }
        });
        let mut res = results.into_inner().unwrap();
        res.sort_by_key(|r| r.0);
        let mut bad = 0;
        for chunk in res.chunks(combos.len()) {
            let id = chunk[0].1;
            let worst = chunk.iter().map(|r| r.6 / r.7).fold(f64::MAX, f64::min);
            let ev: Vec<String> = chunk.iter().map(|r| r.10.to_string()).collect();
            let good = |r: &(usize, &str, f64, f64, Limiter, bool, f64, f64, f64, f64, usize)| r.5 && r.6 > 0.5 * r.7;
            let ok = chunk.iter().skip(1).all(good);
            if !good(&chunk[0]) {
                println!("        (fails at the defaults too)");
            }
            if !ok {
                bad += 1;
            }
            println!("  {} {:<22} lowest rpm in the last second / idle {:.2}; blip peak {:.0} (redline {:.0}); afterfire events per corner [{}]", if ok { "ok " } else { "BAD" }, id, worst, chunk.iter().map(|r| r.8).fold(0.0, f64::max), chunk[0].9, ev.join(" "));
            for r in chunk.iter().filter(|r| !good(r)) {
                println!("        phi {:.2}, overrun fueling {:.0}, {}: finite {}, lowest rpm in the last second {:.0} (idle {:.0})", r.2, r.3, lim_name(r.4), r.5, r.6, r.7);
            }
        }
        println!("  corner order: {}", combos.iter().map(|c| format!("{:.2}/{:.0}/{}", c.0, c.1, if c.2 == Limiter::SparkCut { "spark" } else { "fuel" })).collect::<Vec<_>>().join(" "));
        verdict(&format!("all {} engines finite and running at all {} corners", res.len() / combos.len(), combos.len()), bad == 0);
    }
    if fails > 0 {
        eprintln!("{fails} afterfire check(s) failed");
        std::process::exit(1);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    match args.get(1).map(|s| s.as_str()) {
        Some("list") => {
            for n in spec::names() {
                let s = spec::preset(n).unwrap();
                println!("{:<12} {}", n, s.label);
            }
        }
        Some("spec") => println!("{}", serde_json::to_string_pretty(&load_spec(&args[2])).unwrap()),
        Some("pins") => {
            // For each crank position (slot): which cylinders sit there and where their pins point.
            // Cylinders meant to share a pin must show the same angle.
            let list: Vec<String> = if args.len() > 2 { args[2..].to_vec() } else { spec::names().iter().map(|s| s.to_string()).collect() };
            for n in list {
                let s = load_spec(&n);
                if s.is_wankel() {
                    println!("{:<14} rotary: eccentric phases {:?}", s.name, s.rotor_phase);
                    continue;
                }
                let slots = s.slot.iter().copied().max().unwrap_or(0) + 1;
                let mut parts = Vec::new();
                for sl in 0..slots {
                    let cyls: Vec<usize> = (0..s.n()).filter(|&i| s.slot[i] == sl).collect();
                    let pins: Vec<f64> = cyls.iter().map(|&i| s.pin_angle(i)).collect();
                    let spread = pins.iter().map(|a| ((a - pins[0] + 180.0).rem_euclid(360.0) - 180.0).abs()).fold(0.0f64, f64::max);
                    let verdict = if cyls.len() == 1 { String::new() } else if spread < 1e-6 { " shared pin".to_string() } else { format!(" pins {:.0}° apart", spread) };
                    parts.push(format!("[{}] {}{}", cyls.iter().map(|i| (i + 1).to_string()).collect::<Vec<_>>().join(","), pins.iter().map(|a| format!("{:.0}°", a)).collect::<Vec<_>>().join("/"), verdict));
                }
                let iv = s.intervals();
                let even = iv.iter().all(|x| (x - iv[0]).abs() < 1e-6);
                println!("{:<14} {}\n{:<14}   firing intervals: {}", s.name, parts.join("  "), "", if even { format!("even, every {:.0}°", iv[0]) } else { iv.iter().map(|x| format!("{:.0}", x)).collect::<Vec<_>>().join("-") });
            }
        }
        Some("libtest") => {
            // Checks for what a definition file can express beyond the original presets.
            let mut fails = 0;
            let mut check = |name: &str, ok: bool, detail: String| {
                println!("{} {:<34} {}", if ok { "PASS" } else { "FAIL" }, name, detail);
                if !ok {
                    fails += 1;
                }
            };
            let q = quality("live96");
            let base = load_spec("i4");
            // 1. cam events quoted at a lift land on the lift curve at that lift
            let cam = spec::CamSpec { at_lift: 0.001, ivo: 10.0, ivc: 40.0, evo: 40.0, evc: 10.0, in_lift: 0.010, ex_lift: 0.009 };
            let e = cam.seat_events();
            let lift_at = |open_seat: f64, close_seat: f64, open_q: f64, lmax: f64| {
                let x = (open_seat - open_q) / (open_seat + 180.0 + close_seat);
                lmax * (PI * x).sin().powf(1.4)
            };
            let (li, le) = (lift_at(e.ivo, e.ivc, 10.0, 0.010), lift_at(e.evo, e.evc, 40.0, 0.009));
            check("cam events at 1 mm lift", (li - 0.001).abs() < 1e-9 && (le - 0.001).abs() < 1e-9, format!("curve lift at the quoted angles: intake {:.4} mm, exhaust {:.4} mm; seat-to-seat intake {:.1}/{:.1} from quoted 10/40", li * 1e3, le * 1e3, e.ivo, e.ivc));
            let seat = spec::CamSpec { at_lift: 0.0, ..cam.clone() }.seat_events();
            check("cam events seat to seat", seat.ivo == 10.0 && seat.ivc == 40.0, "at_lift 0 leaves the angles alone".into());
            // 2. one valve instead of two (same diameter) halves the flow area: breathing must drop at speed
            let mut one = base.clone();
            one.in_valves = 1;
            one.ex_valves = 1;
            let (_, _, ve2) = dyno_point(&base, &q, 7000.0);
            let (_, _, ve1) = dyno_point(&one, &q, 7000.0);
            check("valves per cylinder", ve1 < 0.93 * ve2, format!("volumetric efficiency at 7000 rpm: two valves {:.3}, one valve of the same size {:.3}", ve2, ve1));
            // 3. cam switching
            let mut vt = base.clone();
            vt.cam = Some(spec::CamSpec { at_lift: 0.001, ivo: 0.0, ivc: 30.0, evo: 30.0, evc: 0.0, in_lift: 0.0078, ex_lift: 0.0070 });
            vt.cam_high = Some(spec::CamSpec { at_lift: 0.001, ivo: 15.0, ivc: 50.0, evo: 45.0, evc: 12.0, in_lift: 0.0115, ex_lift: 0.0105 });
            vt.cam_switch_rpm = 5800.0;
            let mut lo_only = vt.clone();
            lo_only.cam_high = None;
            let (_, kw_hi, _) = dyno_point(&vt, &q, 8000.0);
            let (_, kw_lo, _) = dyno_point(&lo_only, &q, 8000.0);
            let (t_lo_a, _, _) = dyno_point(&vt, &q, 3000.0);
            let (t_lo_b, _, _) = dyno_point(&lo_only, &q, 3000.0);
            check("cam switch: more power on top", kw_hi > 1.1 * kw_lo, format!("8000 rpm: {:.0} kW on the high cam, {:.0} kW held on the low cam", kw_hi, kw_lo));
            check("cam switch: low cam below it", (t_lo_a - t_lo_b).abs() < 1e-9, format!("3000 rpm torque identical with and without a high cam defined ({:.1} N·m)", t_lo_a));
            {
                let mut sys = System::build(&vt, &q, 5);
                sys.mode = Mode::Free;
                let (mut switches, mut bad_switch, mut finite) = (0, 0, true);
                let mut prev: Vec<bool> = sys.cyls.iter().map(|c| c.cam_high).collect();
                let mut prev_c: Vec<f64> = sys.cyls.iter().map(|c| c.c).collect();
                let (mut seen_high, mut back_low) = (false, false);
                for i in 0..(6.0 * q.rate) as usize {
                    let t = i as f64 / q.rate;
                    sys.throttle_cmd = if (0.5..2.5).contains(&t) { 0.5 } else { 0.0 };
                    let o = sys.advance();
                    finite &= o.iter().all(|v| v.is_finite());
                    for (k, c) in sys.cyls.iter().enumerate() {
                        if c.cam_high != prev[k] {
                            switches += 1;
                            // the change must land on the step where the cycle angle wrapped past firing TDC
                            if !(c.c < prev_c[k]) {
                                bad_switch += 1;
                            }
                            prev[k] = c.cam_high;
                        }
                        prev_c[k] = c.c;
                    }
                    if sys.cyls.iter().all(|c| c.cam_high) {
                        seen_high = true;
                    }
                    if seen_high && t > 3.0 && sys.cyls.iter().all(|c| !c.cam_high) {
                        back_low = true;
                    }
                }
                check("cam switch: only at firing TDC", switches >= 8 && bad_switch == 0 && finite && seen_high && back_low, format!("{} cylinder switches in a rev and lift-off, {} away from firing TDC; went high: {}, came back low: {}, output finite: {}", switches, bad_switch, seen_high, back_low, finite));
            }
            // 4. per-cylinder runner lengths
            let mut rl = base.clone();
            rl.runner_len_cyl = vec![0.30, 0.45, 0.30, 0.45];
            let sys = System::build(&rl.clone().finish().unwrap(), &q, 1);
            let lens: Vec<f64> = sys.pipes.iter().filter(|p| p.kind == "runner").map(|p| p.len).collect();
            check("per-cylinder runner length", lens.len() == 4 && (lens[1] - lens[0] - 0.15).abs() < 1e-9 && (lens[3] - lens[2] - 0.15).abs() < 1e-9, format!("runner pipes built: {:?} m", lens));
            // 5. free bank directions: a radial (one bank per cylinder, one crank pin) and a W (four rows)
            let radial = spec::parse(r#"{"name":"t_radial","label":"test radial","layout":"radial","bore":0.13,"stroke":0.117,"rod":0.25,"cr":6.0,
                "firing":[0, 308.571428571, 617.142857143, 205.714285714, 514.285714286, 102.857142857, 411.428571429],
                "bank":[0,1,2,3,4,5,6],"slot":[0,0,0,0,0,0,0],
                "bank_dirs":[0, 51.428571429, 102.857142857, 154.285714286, 205.714285714, 257.142857143, 308.571428571]}"#);
            match radial {
                Ok(r) => {
                    let pins: Vec<f64> = (0..7).map(|i| r.pin_angle(i)).collect();
                    let iv = r.intervals();
                    let one_pin = pins.iter().all(|a| ((a - pins[0] + 180.0).rem_euclid(360.0) - 180.0).abs() < 1e-6);
                    let even = iv.iter().all(|x| (x - 720.0 / 7.0).abs() < 1e-6);
                    let (tq, _, _) = dyno_point(&r, &q, 2000.0);
                    check("radial: one pin, even firing", one_pin && even && tq > 0.0, format!("pin angles {:?}; intervals every {:.1}°; runs at 2000 rpm with {:.0} N·m", pins.iter().map(|a| a.round()).collect::<Vec<_>>(), iv[0], tq));
                }
                Err(e) => check("radial: one pin, even firing", false, e),
            }
            let w = spec::parse(r#"{"name":"t_w8","label":"test W8","layout":"w","bore":0.084,"stroke":0.09,"rod":0.16,"cr":10.5,
                "firing":[0,90,180,270,360,450,540,630],"bank":[0,1,2,3,0,1,2,3],"slot":[0,0,0,0,1,1,1,1],"bank_dirs":[-43.5,-28.5,28.5,43.5]}"#);
            match w {
                Ok(w) => check("W layout: four bank directions", w.bank_dir(0) == -43.5 && w.bank_dir(3) == 43.5 && System::build(&w, &q, 1).cyls.len() == 8, "bank_dirs replaces bank_angle".into()),
                Err(e) => check("W layout: four bank directions", false, e),
            }
            // 6. the original V, flat and inline pin angles are what they were
            let pins_of = |id: &str| -> Vec<f64> { let s = load_spec(id); (0..s.n()).map(|i| s.pin_angle(i)).collect() };
            check("existing pin angles unchanged", pins_of("v8_cross") == vec![315.0, 315.0, 225.0, 225.0, 45.0, 45.0, 135.0, 135.0] && pins_of("flat6") == vec![270.0, 150.0, 30.0, 90.0, 330.0, 210.0] && pins_of("i4") == vec![0.0, 180.0, 180.0, 0.0], "v8_cross, flat6, i4".into());
            // 7. bad definitions are refused with a reason
            let bad = spec::parse(r#"{"name":"x","label":"x","layout":"inline","bore":0.08,"stroke":0.08,"rod":0.14,"cr":10,"firing":[0,180,360,540],"groups":[[0,1],[2]]}"#);
            check("bad exhaust groups refused", bad.is_err(), bad.err().unwrap_or_default());
            // 8. a minimal definition (required fields only) builds and runs
            let min = spec::parse(r#"{"name":"m","label":"minimal","layout":"inline","bore":0.086,"stroke":0.086,"rod":0.145,"cr":10.5,"firing":[0,540,180,360]}"#);
            match min {
                Ok(m) => {
                    let (tq, _, _) = dyno_point(&m, &q, 4000.0);
                    check("minimal definition runs", tq > 100.0, format!("derived everything else; {:.0} N·m at 4000 rpm", tq));
                }
                Err(e) => check("minimal definition runs", false, e),
            }
            // 9. forced induction: absent or null means naturally aspirated; a blower is refused on engines that cannot take one
            let null_boost = spec::parse(r#"{"name":"m","label":"minimal","layout":"inline","bore":0.086,"stroke":0.086,"rod":0.145,"cr":10.5,"firing":[0,540,180,360],"boost":null}"#);
            check("\"boost\": null parses", null_boost.as_ref().map_or(false, |s| s.boost.is_none()), null_boost.err().unwrap_or_default());
            let itb_boost = spec::parse(r#"{"name":"m","label":"minimal","layout":"inline","bore":0.086,"stroke":0.086,"rod":0.145,"cr":10.5,"firing":[0,540,180,360],"itb":true,"boost":{"kind":"roots"}}"#);
            check("blower on individual throttles refused", itb_boost.is_err(), itb_boost.err().unwrap_or_default());
            // 10. vehicles: a gear_count-only block gets a geometric gearbox and a final drive from
            // the top speed; bad blocks are refused; without a block the generic car is unchanged
            let veh = |b: &str| spec::parse(&format!(r#"{{"name":"m","label":"minimal","layout":"inline","bore":0.086,"stroke":0.086,"rod":0.145,"cr":10.5,"firing":[0,540,180,360],"redline":6000,"vehicle":{b}}}"#));
            match veh(r#"{"name":"Test car","curb_kg":900,"gear_count":4,"top_speed_kmh":150}"#) {
                Ok(m) => {
                    let d = Drive::for_engine(&m, 0, 0.0);
                    let top_kmh = 6000.0 * PI / 30.0 * d.wheel_r / (d.gears[3] * d.final_drive) * 3.6;
                    check("vehicle with gear count only", d.gears.len() == 4 && (d.gears[0] - 3.6).abs() < 1e-12 && (d.gears[3] - 1.0).abs() < 1e-12 && (top_kmh - 150.0).abs() < 1e-9 && d.mass == 980.0, format!("gears {:?}, final drive {:.3}, {:.1} km/h in top at the redline, {:.0} kg", d.gears.iter().map(|g| (g * 1000.0).round() / 1000.0).collect::<Vec<_>>(), d.final_drive, top_kmh, d.mass));
                }
                Err(e) => check("vehicle with gear count only", false, e),
            }
            let wrong = veh(r#"{"name":"Test car","curb_kg":900,"gear_count":5,"gears":[3,2,1]}"#);
            check("vehicle with wrong gear list refused", wrong.is_err(), wrong.err().unwrap_or_default());
            let bad_driven = veh(r#"{"name":"Test car","curb_kg":900,"gear_count":5,"driven":"middle"}"#);
            check("vehicle with unknown driven refused", bad_driven.is_err(), bad_driven.err().unwrap_or_default());
            let mu_of = |b: &str| veh(b).map(|m| engsim::engine::tyre_mu(m.vehicle.as_ref().unwrap())).unwrap_or(f64::NAN);
            let (m1, m2, m3, m4) = (mu_of(r#"{"name":"Old car (1950)","curb_kg":1900,"gear_count":3,"tyre":"8.00-15"}"#), mu_of(r#"{"name":"Car (1988)","curb_kg":1200,"gear_count":5,"tyre":"205/55 VR15"}"#), mu_of(r#"{"name":"Car (2008)","curb_kg":1500,"gear_count":6,"tyre":"P285/35ZR19"}"#), mu_of(r#"{"name":"Car (1978)","curb_kg":900,"gear_count":4,"tyre":"5.20-13"}"#));
            check("tyre grip by era", m1 == 0.85 && m2 == 1.0 && m3 == 1.15 && m4 == 0.85, format!("1950 bias-ply {m1}, 1988 radial {m2}, 2008 ZR {m3}, 1978 bias-ply {m4}"));
            let g = Drive::for_engine(&base, 2, 400.0);
            check("no vehicle: the generic car", base.vehicle.is_none() && g.gears == vec![3.2, 2.05, 1.45, 1.1, 0.88, 0.72] && g.wheel_r == 0.32 && g.cda == 0.68 && g.clutch_launch == 0.0 && g.clutch_gain == 1.5 && g.clutch_nm_per_l == 130.0 && g.eta == 1.0 && g.grip.is_infinite() && g.tc == 1.0, format!("{:.0} kg, final drive {:.3}", g.mass, g.final_drive));
            if fails > 0 {
                eprintln!("{fails} library check(s) failed");
                std::process::exit(1);
            }
        }
        Some("validate") => {
            let write = args.iter().any(|a| a == "--write");
            let ids: Vec<String> = args[2..].iter().filter(|a| !a.starts_with("--")).cloned().collect();
            let ids = if ids.is_empty() { spec::names().iter().map(|s| s.to_string()).collect() } else { ids };
            validate(&ids, write);
        }
        Some("dyno") => {
            let name = &args[2];
            let s = load_spec(name);
            let rpms: Vec<f64> = match flag("--rpms") {
                Some(l) => l.split(',').map(|x| x.parse().unwrap()).collect(),
                None => (1..=8).map(|i| (s.redline * i as f64 / 8.0 / 100.0).round() * 100.0).collect(),
            };
            let ov = flag("--set").map(|j| serde_json::from_str(&j).expect("bad --set JSON")).unwrap_or(serde_json::Value::Null);
            dyno(name, &flag("--quality").unwrap_or_default(), &rpms, &ov);
        }
        Some("selftest") => selftest(),
        Some("trace") => {
            // trace <engine> <rpm> [--set json]: one cycle of cylinder 1 at steady full throttle, every 6 degrees:
            // cylinder, exhaust-port and (two-stroke) crankcase pressure in bar, port openings, gas speed at the exhaust port
            let ov = flag("--set").map(|j| serde_json::from_str(&j).expect("bad --set JSON")).unwrap_or(serde_json::Value::Null);
            let s = spec_with_overrides(&args[2], &ov);
            let rpm: f64 = args[3].parse().unwrap();
            let q = quality("live96");
            let mut sys = System::build(&s, &q, 5);
            sys.mode = Mode::Dyno;
            sys.omega = rpm * PI / 30.0;
            sys.throttle_cmd = 1.0;
            let per_cycle = (s.cycle_deg() / 360.0 * 60.0 / rpm * q.rate) as usize;
            for _ in 0..per_cycle * 40 {
                sys.advance();
            }
            println!("deg  p_cyl  p_export  p_pipe_mid  p_pipe_end  p_crankcase  ex_open  in_open  u_export");
            let mut last = -1i64;
            for _ in 0..per_cycle * 2 {
                sys.advance();
                let c = &sys.cyls[0];
                let deg = (c.c * 180.0 / PI) as i64 / 6;
                if deg != last {
                    last = deg;
                    let p = &sys.pipes[c.ex_pipe];
                    let w = p.prim(0);
                    let cc = if c.cc_m > 0.0 { GM1 * c.cc_e / c.cc_vol() / 1e5 } else { 0.0 };
                    println!("{:>4} {:>6.2} {:>8.2} {:>10.2} {:>10.2} {:>11.2} {:>8.2} {:>8.2} {:>8.0}", deg * 6, c.p() / 1e5, w.p / 1e5, p.prim(p.n / 2).p / 1e5, p.prim(p.n - 2).p / 1e5, cc, c.lift_ex, c.lift_in, w.u);
                }
            }
        }
        Some("dump") => {
            // dump <engine> <quality> <rpm|idle> <secs> <file>: steady full-throttle run, raw f64 [exL, exR, intake, block] per inner sample.
            // Used to check that solver changes leave the sound alone.
            // <rpm> may be "idle": free running at closed throttle from the idle speed, after 2 s to settle.
            let s = load_spec(&args[2]);
            let q = quality(&args[3]);
            let idle = args[4] == "idle";
            let rpm: f64 = if idle { s.idle } else { args[4].parse().unwrap() };
            let secs: f64 = args[5].parse().unwrap();
            let mut sys = System::build(&s, &q, 3);
            sys.mode = if idle { Mode::Free } else { Mode::Dyno };
            sys.omega = rpm * PI / 30.0;
            sys.throttle_cmd = if idle { 0.0 } else { 1.0 };
            for _ in 0..((if idle { 2.0 } else { 0.3 }) * q.rate) as usize {
                sys.advance();
            }
            let mut f = std::io::BufWriter::new(std::fs::File::create(&args[6]).unwrap());
            for _ in 0..(secs * q.rate) as usize {
                for v in sys.advance() {
                    f.write_all(&v.to_le_bytes()).unwrap();
                }
            }
        }
        Some("drive") => {
            // drive <engine> [secs] [--generic] [--quality light|heavy]: the browser's session (live.rs)
            // started in Drive at rest, idling 2.5 s, then full throttle with automatic shifts. Prints
            // speed against time, every gear change and 0-100 km/h. --generic swaps the engine's
            // vehicle for the generic car (docs/physics.md, "Vehicles in Drive").
            let generic = args.iter().any(|a| a == "--generic");
            let qi: u32 = match args.iter().position(|a| a == "--quality").and_then(|i| args.get(i + 1)).map(|s| s.as_str()) {
                Some("heavy") => 1,
                _ => 0,
            };
            let secs: f64 = args.get(3).and_then(|x| x.parse().ok()).unwrap_or(12.0);
            drive_run(&args[2], secs, generic, qi);
        }
        Some("live") => {
            // live <engine> <light|heavy|quality index> [secs] [idle|wot]: the browser's real-time session
            // (sim/src/live.rs) run natively, 128-frame blocks. Built with `--features prof` it also
            // prints where the time went (sim/src/prof.rs).
            live_profile(&args[2], args.get(3).map(|s| s.as_str()).unwrap_or("light"), args.get(4).and_then(|x| x.parse().ok()).unwrap_or(2.0), args.get(5).map(|s| s.as_str()) == Some("wot"));
        }
        Some("bench") => {
            // bench <engine> [draft|final] [rpm] [level]: level 0 = plain timing, 1 = section timers, 2 = also inside the pipe solver
            let s = load_spec(&args[2]);
            let qn = args.get(3).cloned().unwrap_or_default();
            let rpm: f64 = args.get(4).and_then(|x| x.parse().ok()).unwrap_or(4000.0);
            let level: u8 = args.get(5).and_then(|x| x.parse().ok()).unwrap_or(0);
            let q = quality(&qn);
            let mut sys = System::build(&s, &q, 3);
            sys.mode = Mode::Dyno;
            sys.omega = rpm * PI / 30.0;
            sys.throttle_cmd = 1.0;
            for _ in 0..(0.3 * q.rate) as usize {
                sys.advance();
            }
            sys.prof_on = level >= 1;
            for p in sys.pipes.iter_mut() {
                p.prof_on = level >= 2;
            }
            let secs = 1.0;
            let t0 = std::time::Instant::now();
            let mut acc = 0.0;
            for _ in 0..(secs * q.rate) as usize {
                acc += sys.advance()[0];
            }
            let el = t0.elapsed().as_secs_f64();
            let cells: usize = sys.pipes.iter().map(|p| p.n).sum();
            let steps = secs * q.rate;
            println!("{} {} @ {:.0} rpm WOT: {:.2}x real time ({:.2} s per 1 s audio), {} pipes, {} cells, {:.0} steps/s, {:.1} ns per cell-step, max substeps {} [{:.1}]", s.name, if qn.is_empty() { "draft" } else { qn.as_str() }, rpm, el / secs, el, sys.pipes.len(), cells, steps, el * 1e9 / (cells as f64 * steps), sys.max_sub, acc.abs().min(1.0));
            {
                let mut kinds: std::collections::BTreeMap<String, (f64, f64)> = Default::default();
                for p in &sys.pipes {
                    let e = kinds.entry(p.kind.clone()).or_default();
                    e.0 = e.0.max(p.smax_seen);
                    e.1 = e.1.max(p.smax_seen / q.rate / p.dx);
                }
                let extra: u64 = sys.pipes.iter().map(|p| p.substeps * p.n as u64).sum();
                println!("  extra cell-steps from per-pipe substepping: {:.1}%", 100.0 * extra as f64 / (cells as f64 * steps));
                println!("  peak wave speed m/s (peak CFL): {}", kinds.iter().map(|(k, v)| format!("{} {:.0} ({:.2})", k, v.0, v.1)).collect::<Vec<_>>().join(", "));
            }
            if level >= 1 {
                let tot: u64 = sys.prof.iter().sum();
                let names = ["pipes", "orifices", "cylinders", "volumes+crank", "mics+noise"];
                for (n, v) in names.iter().zip(sys.prof.iter()) {
                    println!("  {:<14} {:>6.1}%  {:>7.3} s", n, 100.0 * *v as f64 / tot as f64, *v as f64 * 1e-9);
                }
                println!("  (timed total {:.3} s; remainder is timer overhead and the control code)", tot as f64 * 1e-9);
            }
            if level >= 2 {
                let mut kinds: std::collections::BTreeMap<String, (usize, u64, [u64; 4])> = Default::default();
                for p in &sys.pipes {
                    let e = kinds.entry(p.kind.clone()).or_default();
                    e.0 += p.n;
                    e.1 += p.prof.iter().sum::<u64>();
                    for k in 0..4 {
                        e.2[k] += p.prof[k];
                    }
                }
                let ptot: u64 = kinds.values().map(|v| v.1).sum();
                println!("  inside the pipe solver, by pipe kind (cells, share of pipe time):");
                for (k, v) in &kinds {
                    println!("    {:<10} {:>5} cells  {:>5.1}%", k, v.0, 100.0 * v.1 as f64 / ptot as f64);
                }
                let mut ph = [0u64; 4];
                for v in kinds.values() {
                    for k in 0..4 {
                        ph[k] += v.2[k];
                    }
                }
                let pn = ["primitives+wave speed", "reconstruction", "Riemann fluxes+ends", "update+friction+heat"];
                println!("  inside the pipe solver, by phase:");
                for k in 0..4 {
                    println!("    {:<22} {:>5.1}%", pn[k], 100.0 * ph[k] as f64 / ptot as f64);
                }
            }
        }
        Some("idle") => {
            // blip the throttle, then report how well each engine settles back to idle
            let list: Vec<String> = if args.len() > 2 { args[2..].to_vec() } else { spec::names().iter().map(|s| s.to_string()).collect() };
            for name in list {
                let s = load_spec(&name);
                let q = quality(&std::env::var("ENGSIM_QUALITY").unwrap_or_default());
                let mut sys = System::build(&s, &q, 7);
                sys.mode = Mode::Free;
                let (mut lo, mut hi, mut sum, mut n, mut peak) = (f64::MAX, 0.0f64, 0.0, 0.0, 0.0f64);
                for i in 0..(7.0 * q.rate) as usize {
                    let t = i as f64 / q.rate;
                    sys.throttle_cmd = if (1.5..1.75).contains(&t) { 0.4 } else { 0.0 };
                    sys.advance();
                    let r = sys.rpm();
                    peak = peak.max(r);
                    if t > 1.9 {
                        lo = lo.min(r);
                        hi = hi.max(r);
                    }
                    if t > 4.0 {
                        sum += r;
                        n += 1.0;
                    }
                }
                println!("{:<12} target {:>5.0}  settled mean {:>5.0}  min after blip {:>5.0}  max {:>5.0}  blip peak {:>6.0}", name, s.idle, sum / n, lo, hi, peak);
            }
        }
        Some("probe") => {
            let s = load_spec(&args[2]);
            let q = quality("");
            let mut sys = System::build(&s, &q, 7);
            sys.mode = Mode::Free;
            println!("t,rpm,throttle,map_bar,cut,idle_mode,torque,air0_frac,q0_J,cyl0_T");
            for i in 0..(4.2 * q.rate) as usize {
                let t = i as f64 / q.rate;
                sys.throttle_cmd = if (1.5..1.75).contains(&t) { 0.4 } else { 0.0 };
                sys.advance();
                if t > 1.7 && i % (q.rate as usize / 25) == 0 {
                    let map = sys.plenum.map(|p| sys.vols[p].p() / 1e5).unwrap_or(0.0);
                    let c = &sys.cyls[0];
                    println!("{:.2},{:.0},{:.4},{:.3},{},{},{:.0},{:.3},{:.0},{:.0}", t, sys.rpm(), sys.throttle_eff, map, sys.fuel_cut as u8, sys.idle_mode as u8, sys.torque, c.air_last / (RHO_ATM * c.v_d), c.q_dbg, c.t());
                }
            }
        }
        Some("render") => render(&args[2], &args[3]),
        Some("liftoff") => liftcheck::run(&args[2..]),
        Some("boost") => boostcheck::run(&args[2..]),
        Some("turbo") => turbocheck::run(&args[2..]),
        Some("afterfire") => {
            let qflag = flag("--quality");
            let parts: Vec<String> = args[2..].iter().filter(|a| !a.starts_with("--") && Some(a.as_str()) != qflag.as_deref()).cloned().collect();
            afterfire_checks(&qflag.unwrap_or_else(|| "live96+W".into()), &parts);
        }
        _ => eprintln!("usage: engsim list | spec <engine|file.json> | pins [engine...] | validate [engine...] [--write] | libtest | dyno <engine> [--rpms a,b] [--quality final] [--set '{{json}}'] | selftest | render <scene.json> <out-dir> | afterfire [a..f] | liftoff <engine|table|stats|all|drive <engine>|stress> [quality] [level] | boost <rig|curve|whine|stability|trace|all> ..."),
    }
}

fn drive_run(engine: &str, secs: f64, generic: bool, qi: u32) {
    use engsim::live::*;
    let s = load_spec(engine);
    let ov = if generic { serde_json::json!({"vehicle": null}) } else { serde_json::json!({}) };
    let req = serde_json::json!({"engine": engine, "overrides": ov}).to_string();
    let l = unsafe { es_new(req.as_ptr(), req.len(), qi, 0.0) };
    assert!(!l.is_null(), "es_new failed");
    let h: serde_json::Value = unsafe { serde_json::from_slice(std::slice::from_raw_parts(es_header_ptr(l), es_header_len(l))).unwrap() };
    let car = match h.get("vehicle") {
        Some(v) => format!("{} ({}, {:.0} kg, {} gears)", v["name"].as_str().unwrap_or(""), v["kind"].as_str().unwrap_or(""), v["mass_kg"].as_f64().unwrap_or(0.0), v["gear_count"]),
        None => {
            let d = Drive::for_engine(&engsim::spec::EngineSpec { vehicle: None, ..s.clone() }, 0, 0.0);
            format!("generic car ({:.0} kg, {} gears)", d.mass, d.gears.len())
        }
    };
    println!("{engine} in {car}, quality {}", h["quality"].as_str().unwrap_or(""));
    if !generic {
        if let Some(v) = &s.vehicle {
            let d = Drive::for_engine(&s, 0, 0.0);
            let top = s.redline * PI / 30.0 * d.wheel_r / (d.gears[d.gears.len() - 1] * d.final_drive) * 3.6;
            println!("  top gear at the redline: {:.0} km/h; published top speed: {}; overall ratios {:?}", top, v.top_speed_kmh.map_or("none".to_string(), |t| format!("{t:.0} km/h")), d.gears.iter().map(|g| (g * d.final_drive * 100.0).round() / 100.0).collect::<Vec<_>>());
        }
    }
    unsafe { es_set_drive(l, 1) };
    let mut out = vec![0.0f32; 256];
    let block = 128.0 / 48000.0;
    let frame = |l| unsafe { std::slice::from_raw_parts(es_frame_ptr(l), 8).to_vec() };
    for _ in 0..(2.5 / block) as usize {
        unsafe { es_process(l, out.as_mut_ptr(), 128) };
    }
    let f = frame(l);
    println!("  idle in Drive: {:.0} rpm, gear {}, {:.2} km/h", f[2], f[5], f[6] * 3.6);
    unsafe { es_set_throttle(l, 1.0) };
    let (mut t, mut gear, mut t100, mut marks) = (0.0f64, f[5], f64::NAN, vec![]);
    let mut t60 = f64::NAN;
    let mut min_rpm = f64::MAX;
    let mut next = 1.0;
    let (mut next_r, mut rpms) = (0.5, vec![]);
    while t < secs {
        unsafe { es_process(l, out.as_mut_ptr(), 128) };
        t += block;
        let f = frame(l);
        let (rpm, g, kmh) = (f[2] as f64, f[5], f[6] as f64 * 3.6);
        if t < 1.5 {
            min_rpm = min_rpm.min(rpm);
        }
        if g != gear {
            println!("  {:5.2} s  gear {} -> {} at {:.0} km/h, {:.0} rpm", t, gear, g, kmh, rpm);
            gear = g;
        }
        if t60.is_nan() && kmh >= 96.56 {
            t60 = t;
        }
        if t100.is_nan() && kmh >= 100.0 {
            t100 = t;
        }
        if t >= next_r && next_r <= 8.0 {
            // engine speed against the road speed in this gear: tyre slip (or clutch slip at launch)
            let d0 = Drive::for_engine(&s, 0, 0.0);
            let gi = ((g as usize).max(1) - 1).min(d0.gears.len() - 1);
            let w_road = f[6] as f64 * d0.gears[gi] * d0.final_drive / d0.wheel_r;
            let over = if w_road > 1.0 && !generic { format!("({:+.0}%)", 100.0 * (rpm * PI / 30.0 / w_road - 1.0)) } else { String::new() };
            rpms.push(format!("{:.0}{}", rpm, over));
            next_r += 0.5;
        }
        if t >= next {
            marks.push(format!("{:.0}s {:.0}", next, kmh));
            next += 1.0;
        }
    }
    println!("  lowest engine speed in the first 1.5 s of throttle: {:.0} rpm", min_rpm);
    println!("  km/h: {}", marks.join(" | "));
    println!("  rpm every 0.5 s to 8 s (in brackets: engine speed over road speed in that gear): {}", rpms.join(" "));
    println!("  0-60 mph (96.6 km/h): {}", if t60.is_nan() { format!("not within {secs:.0} s") } else { format!("{t60:.1} s") });
    println!("  0-100 km/h: {}", if t100.is_nan() { format!("not within {secs:.0} s") } else { format!("{t100:.1} s") });
    unsafe { es_drop(l) };
}

fn live_profile(engine: &str, quality: &str, secs: f64, wot: bool) {
    use engsim::prof::{self, cat, cnt};
    let qi: u32 = match quality {
        "light" => 0,
        "heavy" => 1,
        q => q.parse().expect("quality: light, heavy or an index into live.rs QUALITIES"),
    };
    let s = load_spec(engine);
    let req = serde_json::json!({"engine": engine, "overrides": {}}).to_string();
    let start = if wot { 0.7 * s.redline } else { 0.0 };
    let l = unsafe { engsim::live::es_new(req.as_ptr(), req.len(), qi, start) };
    assert!(!l.is_null(), "es_new failed");
    unsafe { engsim::live::es_set_throttle(l, if wot { 1.0 } else { 0.0 }) };
    let mut out = vec![0.0f32; 256];
    let settle = if wot { 0.3 } else { 2.0 };
    for _ in 0..(settle * 48000.0 / 128.0) as usize {
        unsafe { engsim::live::es_process(l, out.as_mut_ptr(), 128) };
    }
    prof::take();
    let blocks = (secs * 48000.0 / 128.0) as usize;
    let t0 = std::time::Instant::now();
    for _ in 0..blocks {
        unsafe { engsim::live::es_process(l, out.as_mut_ptr(), 128) };
    }
    let el = t0.elapsed().as_secs_f64();
    let audio = blocks as f64 * 128.0 / 48000.0;
    let rpm = unsafe { engsim::live::es_rpm(l) };
    println!("{engine} {quality} {}: {:.3} s per s of sound, {:.0} rpm", if wot { "wot" } else { "idle" }, el / audio, rpm);
    if prof::ON {
        let (mut a, c) = prof::take();
        // the pipe-loop slot holds the whole loop; leave only what is outside the two solvers
        let inner: f64 = (cat::GRID_PRIM..=cat::WAVE).map(|i| a[i]).sum();
        a[cat::PIPE_OTHER] -= inner;
        let tot: f64 = a[..cat::RAD].iter().sum();
        println!("  timed {:.3} s per s (untimed run above includes timer overhead too)", tot / audio);
        for i in 0..cat::N {
            if cat::NAMES[i].is_empty() || a[i] == 0.0 {
                continue;
            }
            println!("  {:<34} {:>6.1}%  {:>8.4} s/s", cat::NAMES[i], 100.0 * a[i] / tot, a[i] / audio);
        }
        for i in 0..cnt::N {
            if cnt::NAMES[i].is_empty() {
                continue;
            }
            println!("  {:<40} {:>12.0} per s", cnt::NAMES[i], c[i] as f64 / audio);
        }
        if c[cnt::RAD_CALLS] > 0 {
            println!("  residual evaluations per radiation solve {:.2}", c[cnt::RAD_EVALS] as f64 / c[cnt::RAD_CALLS] as f64);
        }
    }
    unsafe { engsim::live::es_drop(l) };
}
