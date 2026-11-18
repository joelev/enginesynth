//! Checks for the superchargers: `engsim boost ...` (usage in `run`). Every check is numeric;
//! nobody has listened. The generic blower below is a test fixture only: it is never a library
//! engine and never reaches the app.

use super::*;
use engsim::boost::*;

/// A copy of `s` with a generic Roots blower (throttle before it, with a bypass valve), sized by
/// the derivation rules in `BoostSpec::finish` for `boost_bar` at 70% of the redline.
pub fn generic(s: &spec::EngineSpec, boost_bar: f64) -> spec::EngineSpec {
    let layout = std::env::var("ENGSIM_BOOST_LAYOUT").unwrap_or_default();
    let mut v = s.clone();
    let j = match layout.as_str() {
        // the other installation: blower ahead of the throttle, an intercooler and a charge pipe
        // to the throttle (the pipe is solved like the others), bypass valve
        "after" => serde_json::json!({"kind": "roots", "induction": "test fixture", "boost_bar": boost_bar, "throttle": "after", "bypass": {}, "intercooler": {}, "charge_len": 0.6, "charge_d": 0.06}),
        "screw" => serde_json::json!({"kind": "twin_screw", "induction": "test fixture", "boost_bar": boost_bar, "lobes": 4, "bypass": {}}),
        _ => serde_json::json!({"kind": "roots", "induction": "test fixture", "boost_bar": boost_bar, "bypass": {}}),
    };
    v.boost = Some(serde_json::from_value(j).unwrap());
    v.finish().expect("generic blower")
}

fn without(s: &spec::EngineSpec) -> spec::EngineSpec {
    let mut v = s.clone();
    v.boost = None;
    v
}

/// The spec under test: a library engine with its own boost, or any plenum engine given a generic
/// blower with `--generic`.
fn engine_arg(name: &str, generic_bar: Option<f64>) -> spec::EngineSpec {
    let s = load_spec(name);
    match generic_bar {
        Some(b) => generic(&s, b),
        None => {
            if s.boost.is_none() {
                eprintln!("{name} has no supercharger; add --generic <bar> to test it with the generic test blower");
                std::process::exit(2);
            }
            s
        }
    }
}

/// Amplitude of the component of `x` at frequency f (Hann window).
fn amp_at(x: &[f64], rate: f64, f: f64) -> f64 {
    let n = x.len();
    let (mut re, mut im, mut ws) = (0.0, 0.0, 0.0);
    let w0 = 2.0 * PI * f / rate;
    // rotate a phasor instead of calling sin and cos per sample
    let (cr, ci) = (w0.cos(), -w0.sin());
    let (mut pr, mut pi) = (1.0f64, 0.0f64);
    for (k, v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (2.0 * PI * k as f64 / (n - 1) as f64).cos();
        re += v * w * pr;
        im += v * w * pi;
        ws += w;
        let t = pr * cr - pi * ci;
        pi = pr * ci + pi * cr;
        pr = t;
        if k % 4096 == 4095 {
            let r = (pr * pr + pi * pi).sqrt();
            pr /= r;
            pi /= r;
        }
    }
    2.0 * (re * re + im * im).sqrt() / ws
}

fn db(a: f64) -> f64 {
    20.0 * a.max(1e-30).log10()
}

fn rig_geom(screw: bool, leak: f64) -> BlowerGeom {
    let vd = 1.47e-3;
    BlowerGeom { kind_screw: screw, displacement: vd, ratio: 1.0, rotors: 2, lobes: if screw { 4 } else { 3 }, helix_deg: if screw { 300.0 } else { 60.0 }, vi: if screw { 1.8 } else { 1.0 }, carry: 0.03, a_port: 0.6 * (vd / if screw { 4.0 } else { 6.0 }).powf(2.0 / 3.0), a_leak: leak, mech_eff: 1.0, drag: 0.0 }
}

/// Blower alone between two very large volumes held at a pressure ratio.
fn rig_flow(screw: bool, pr: f64, rotor_rpm: f64, rate: f64) -> (f64, f64, f64) {
    let mut vols = vec![Volume::new("u", 1.0, T_ATM), Volume::new("d", 1.0, T_ATM)];
    vols[1].set(P_ATM * pr, T_ATM * pow_gm1_g(pr));
    let mut b = Blower::new(0, 1, &rig_geom(screw, 0.0));
    let dt = 1.0 / rate;
    let w = rotor_rpm * PI / 30.0;
    let mut theta = 0.0;
    let steps_rev = (2.0 * PI / (w * dt)) as usize;
    for _ in 0..steps_rev * 3 {
        b.step(&mut vols, theta, w * dt, dt);
        theta += w * dt;
        for v in vols.iter_mut() {
            v.apply();
        }
        vols[1].set(P_ATM * pr, T_ATM * pow_gm1_g(pr));
        vols[0].set(P_ATM, T_ATM);
    }
    let (m0, w0, r0) = (b.m_out, b.w_gas, b.revs);
    for _ in 0..steps_rev * 20 {
        b.step(&mut vols, theta, w * dt, dt);
        theta += w * dt;
        for v in vols.iter_mut() {
            v.apply();
        }
        vols[1].set(P_ATM * pr, T_ATM * pow_gm1_g(pr));
        vols[0].set(P_ATM, T_ATM);
    }
    let revs = b.revs - r0;
    let delivered = (b.m_out - m0) / revs / (RHO_ATM * 1.47e-3);
    let work = (b.w_gas - w0) / revs;
    (delivered, work, (P_ATM * pr - P_ATM) * 1.47e-3)
}

fn rig() {
    println!("Rig: an M90-sized blower (1.47 L per revolution, 2 rotors x 3 lobes, 60 degree helix; twin-screw 4 lobes, Vi 1.8) between two 1 m^3 volumes held at the pressure ratio, no leakage, 48 kHz steps.");
    println!("delivered = mass into the outlet per revolution / (inlet density x displacement); work = rotor work on the gas per revolution, against (p_out - p_in) x displacement\n");
    println!("work above flow loss = work minus the same blower's work at pressure ratio 1 and the same speed (the port losses), against dp*V");
    println!("efficiency = isentropic work for the delivered mass and pressure ratio (the model's gas, gamma 4/3) / rotor work\n");
    println!("{:<10} {:>4} {:>8} {:>10} {:>11} {:>11} {:>10} {:>8} {:>10}", "kind", "PR", "rotor", "delivered", "work J/rev", "dp*V J/rev", "above/dpV", "w/dpV", "efficiency");
    let mut worst_vol: f64 = 0.0;
    let mut worst_work: f64 = 0.0;
    for screw in [false, true] {
        let speeds = [500.0, 2000.0, 6000.0, 14000.0];
        let flow_loss: Vec<f64> = speeds.iter().map(|&n| rig_flow(screw, 1.0, n, 48_000.0).1).collect();
        for &pr in &[1.0, 1.05, 1.2, 1.5, 2.0] {
            for (k, &n) in speeds.iter().enumerate() {
                let (del, w, ideal) = rig_flow(screw, pr, n, 48_000.0);
                let ratio = if ideal > 0.0 { w / ideal } else { f64::NAN };
                let above = if ideal > 0.0 { (w - flow_loss[k]) / ideal } else { f64::NAN };
                let w_is = del * RHO_ATM * 1.47e-3 * CP * T_ATM * (pow_gm1_g(pr) - 1.0);
                println!("{:<10} {:>4.2} {:>8.0} {:>10.4} {:>11.2} {:>11.2} {:>10.4} {:>8.3} {:>10.3}", if screw { "twin_screw" } else { "roots" }, pr, n, del, w, ideal, above, ratio, w_is / w);
                if !screw && n == 500.0 && pr <= 1.05 {
                    worst_vol = worst_vol.max((del - 1.0).abs());
                }
                if !screw && n == 500.0 && pr >= 1.05 {
                    worst_work = worst_work.max((above - 1.0).abs());
                }
            }
        }
    }
    println!("\n{} Roots delivered volume at low pressure ratio and speed within 5% of displacement (worst {:+.2}%)", if worst_vol < 0.05 { "PASS" } else { "FAIL" }, 100.0 * worst_vol);
    println!("{} Roots backflow work at 500 rpm (work above the port losses) within 1% of (p_out - p_in) x displacement (worst {:.2}%)", if worst_work < 0.01 { "PASS" } else { "FAIL" }, 100.0 * worst_work);
    println!("     (Wikipedia, Roots blower: about 70% efficient up to a pressure ratio of two, 50 to 58% at 15 psi)\n");

    // closed rig: mass and energy conservation with leakage, bypass and pockets all working
    let mut vols = vec![Volume::new("u", 2.0e-3, T_ATM), Volume::new("d", 2.0e-3, T_ATM)];
    let mut b = Blower::new(0, 1, &rig_geom(false, 1.3e-4));
    // (a bypass held fully open: its thresholds are far above anything the rig reaches)
    b.bypass = Some(Bypass { area: 3e-4, p_open: P_ATM + 1e7, p_shut: P_ATM + 2e7, tau: 0.02, sense: 1, x: 0.0, mdot: 0.0 });
    let total = |vols: &[Volume], b: &Blower| {
        let (pm, pe) = b.held();
        (vols.iter().map(|v| v.m).sum::<f64>() + pm, vols.iter().map(|v| v.e).sum::<f64>() + pe)
    };
    let (m0, e0) = total(&vols, &b);
    let dt = 1.0 / 48_000.0;
    let w = 12_000.0 * PI / 30.0;
    let mut theta = 0.0;
    let mut finite = true;
    for _ in 0..9_600 {
        b.step(&mut vols, theta, w * dt, dt);
        theta += w * dt;
        for v in vols.iter_mut() {
            v.apply();
        }
        finite &= vols.iter().all(|v| v.m.is_finite() && v.e.is_finite());
    }
    let (m1, e1) = total(&vols, &b);
    let dm = (m1 - m0) / m0;
    let de = (e1 - e0 - b.w_gas) / e0;
 println!("\nClosed rig (2 L each side, leakage 1.3 cm^2, bypass valve 3 cm^2 held open), 12,000 rpm for 0.2 s with no heat loss: outlet {:+.1} kPa, inlet {:+.1} kPa, bypass {:.2} open", (vols[1].p() - P_ATM) / 1e3, (vols[0].p() - P_ATM) / 1e3, b.bypass.as_ref().unwrap().x);
    println!("{} mass conserved: relative change {:.1e}", if dm.abs() < 1e-12 && finite { "PASS" } else { "FAIL" }, dm);
    println!("{} energy conserved: (energy change - rotor work) / energy {:.1e} (rotor work {:.0} J)", if de.abs() < 1e-10 && finite { "PASS" } else { "FAIL" }, de, b.w_gas);
}

/// Steady full throttle at one speed on the dyno: torque, power, and the blower's numbers.
pub struct Point {
    pub torque: f64,
    pub kw: f64,
    pub boost_kpa: f64,
    pub inlet_kpa: f64,
    pub drive_kw: f64,
    pub bypass: f64,
    pub blower_ve: f64,
    pub air_kg_s: f64,
}

pub fn point(s: &spec::EngineSpec, q: &Quality, rpm: f64, throttle: f64) -> Point {
    let mut sys = System::build(s, q, 12345);
    sys.mode = Mode::Dyno;
    sys.omega = rpm * PI / 30.0;
    sys.throttle_cmd = throttle;
    let per_cycle = (120.0 / rpm * q.rate) as usize;
    for _ in 0..per_cycle * 14 {
        sys.advance();
    }
    sys.reset_stats();
    let (mo0, rv0) = sys.machines.first().map_or((0.0, 0.0), |m| (m.m_out - m.m_leak, m.revs));
    let mut pin = 0.0;
    let mut bp = 0.0;
    let n = per_cycle * 8;
    for _ in 0..n {
        sys.advance();
        if let Some(m) = sys.machines.first() {
            pin += sys.vols[m.u].p();
            bp += m.bypass.as_ref().map_or(0.0, |b| b.x);
        }
    }
    let st = sys.stats;
    let torque = (st.work - st.fric_work - st.drive_work) / st.angle;
    let secs = n as f64 / q.rate;
    let (ve, inlet) = match sys.machines.first() {
        Some(m) => {
            let rho_in = pin / n as f64 / (R_GAS * sys.vols[m.u].t());
            (((m.m_out - m.m_leak) - mo0) / ((m.revs - rv0) * m.v_p * m.pockets_per_rev() as f64 * rho_in), pin / n as f64)
        }
        None => (f64::NAN, P_ATM),
    };
    Point {
        torque,
        kw: torque * rpm * PI / 30.0 / 1000.0,
        boost_kpa: (st.p_man / st.n - P_ATM) / 1e3,
        inlet_kpa: (inlet - P_ATM) / 1e3,
        drive_kw: st.drive_work / secs / 1e3,
        bypass: bp / n as f64,
        blower_ve: ve,
        air_kg_s: st.air / secs,
    }
}

fn curve(s: &spec::EngineSpec, q: &Quality, rpms: &[f64]) {
    let b = s.boost.as_ref().unwrap();
    println!("{} — {}: {} blower, {:.2} L/rev, drive ratio {:.3}, throttle {} the blower{}{}", s.name, s.label, b.kind, b.displacement * 1e3, b.drive_ratio, b.throttle, if b.bypass.is_some() { ", bypass valve" } else { "" }, if b.intercooler.is_some() { ", intercooler" } else { "" });
    let na = without(s);
    println!("{:>6} {:>8} {:>7} {:>9} {:>7} {:>9} {:>8} {:>7} {:>7} {:>8} {:>8}", "rpm", "N·m", "kW", "boost kPa", "psi", "inlet kPa", "drive kW", "bypass", "blwVE", "NA N·m", "NA kW");
    for &rpm in rpms {
        let p = point(s, q, rpm, 1.0);
        let pn = point(&na, q, rpm, 1.0);
        println!("{:>6.0} {:>8.1} {:>7.1} {:>9.1} {:>7.2} {:>9.1} {:>8.1} {:>7.2} {:>7.3} {:>8.1} {:>8.1}", rpm, p.torque, p.kw, p.boost_kpa, p.boost_kpa / 6.894757, p.inlet_kpa, p.drive_kw, p.bypass, p.blower_ve, pn.torque, pn.kw);
    }
}

/// Spectrum check: the whine must sit at pockets per revolution x rotor speed.
fn whine(s: &spec::EngineSpec, q: &Quality, rpm: f64, secs: f64) -> bool {
    let b = s.boost.as_ref().unwrap();
    let run = |s: &spec::EngineSpec| {
        let mut sys = System::build(s, q, 3);
        sys.mode = Mode::Dyno;
        sys.omega = rpm * PI / 30.0;
        sys.throttle_cmd = 1.0;
        for _ in 0..(0.5 * q.rate) as usize {
            sys.advance();
        }
        let n = (secs * q.rate) as usize;
        let (mut intake, mut exhaust, mut plen, mut inlet) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
        for _ in 0..n {
            let o = sys.advance();
            intake.push(o[2]);
            exhaust.push(o[0] + o[1]);
            plen.push(sys.plenum.map_or(0.0, |i| sys.vols[i].p()));
            inlet.push(sys.machines.first().map_or(0.0, |m| sys.vols[m.u].p()));
        }
        let ppr = sys.machines.first().map_or(0, |m| m.pockets_per_rev());
        (intake, exhaust, plen, inlet, ppr)
    };
    let (intake, exhaust, plen, inlet, ppr) = run(s);
    let f_pred = ppr as f64 * b.drive_ratio * rpm / 60.0;
    let fire = s.n() as f64 * rpm / 120.0;
    println!("{} at {:.0} rpm, full throttle on the dyno, {} ({} s): predicted lobe-pass {} pockets x {:.3} x {:.1} rev/s = {:.1} Hz (firing frequency {:.1} Hz)", s.name, rpm, if q.wave { "characteristic pipes" } else { "grid pipes" }, secs, ppr, b.drive_ratio, rpm / 60.0, f_pred, fire);
    // the strongest line in a band around the prediction, at 0.5 Hz steps
    let mut ok = true;
    for (name, x) in [("intake mouth", &intake), ("exhaust", &exhaust), ("plenum pressure", &plen), ("blower inlet pressure", &inlet)] {
        let lo = 0.8 * f_pred;
        let hi = 1.2 * f_pred;
        let mut best = (0.0, 0.0);
        let mut f = lo;
        while f <= hi {
            let a = amp_at(x, q.rate, f);
            if a > best.1 {
                best = (f, a);
            }
            f += 0.5;
        }
        // the engine's own lines nearby: every half order of the crank in the band
        let half = rpm / 120.0;
        let mut eng: f64 = 0.0;
        let mut k = (lo / half).ceil();
        while k * half <= hi {
            eng = eng.max(amp_at(x, q.rate, k * half));
            k += 1.0;
        }
        let at = amp_at(x, q.rate, f_pred);
        let h2 = amp_at(x, q.rate, 2.0 * f_pred);
        let err = (best.0 - f_pred) / f_pred;
        let mean = x.iter().sum::<f64>() / x.len() as f64;
        let rms = (x.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / x.len() as f64).sqrt();
        let unit = if name.ends_with("pressure") { "Pa" } else { "Pa at 1 m" };
        println!("  {:<22} strongest line in 0.8-1.2 x prediction {:.1} Hz ({:+.2}%), {:+.1} dB against the strongest engine half-order in that band; the line {:.3} {} ({:+.1} dB against the whole signal's rms), 2nd harmonic {:.3}", name, best.0, 100.0 * err, db(best.1 / eng), at, unit, db(at / std::f64::consts::SQRT_2 / rms), h2);
        if name == "intake mouth" || name == "plenum pressure" {
            ok &= err.abs() < 0.005 && best.1 > eng;
        }
    }
    // pressure ripple at the lobe-pass harmonics, rms
    let ripple = |x: &[f64]| (1..=4).map(|k| amp_at(x, q.rate, k as f64 * f_pred).powi(2) / 2.0).sum::<f64>().sqrt();
    let mean_pl = plen.iter().sum::<f64>() / plen.len() as f64;
    let mean_in = inlet.iter().sum::<f64>() / inlet.len() as f64;
    println!("  lobe-pass ripple (first four harmonics, rms): plenum {:.2} kPa at pressure ratio {:.2} over the blower inlet; blower inlet {:.2} kPa. (US 4,827,890 quotes about 14 kPa rms at the outlet port and 3.4 kPa at the inlet at PR 1.5; the plenum here is a lumped volume, not the port.)", ripple(&plen) / 1e3, mean_pl / mean_in, ripple(&inlet) / 1e3);
    ok
}

/// Free running: idle, full throttle into the limiter, lift, back to idle. Returns a summary line and pass/fail.
fn rev_run(s: &spec::EngineSpec, q: &Quality, seed: u64) -> (String, bool, Vec<[f64; 4]>) {
    let mut sys = System::build(s, q, seed);
    sys.mode = Mode::Free;
    let (mut finite, mut top, mut lo_after, mut sum, mut n, mut peak_boost, mut min_boost) = (true, 0.0f64, f64::MAX, 0.0, 0.0, f64::MIN, f64::MAX);
    let mut trace = Vec::new();
    let total = 8.0;
    for i in 0..(total * q.rate) as usize {
        let t = i as f64 / q.rate;
        sys.throttle_cmd = if (1.0..3.5).contains(&t) { 1.0 } else { 0.0 };
        let o = sys.advance();
        finite &= o.iter().all(|v| v.is_finite());
        let r = sys.rpm();
        let bst = sys.plenum.map_or(0.0, |i| sys.vols[i].p()) - P_ATM;
        if (1.0..3.6).contains(&t) {
            top = top.max(r);
        }
        if t > 3.6 {
            lo_after = lo_after.min(r);
        }
        peak_boost = peak_boost.max(bst);
        min_boost = min_boost.min(bst);
        if t > total - 1.5 {
            sum += r;
            n += 1.0;
        }
        if i % (q.rate as usize / 100) == 0 {
            trace.push([t, r, bst / 1e3, sys.machines.first().map_or(0.0, |m| m.bypass.as_ref().map_or(0.0, |b| b.x))]);
        }
    }
    let settled = sum / n;
    let ok = finite && settled.is_finite() && (settled / s.idle - 1.0).abs() < 0.25 && lo_after > 0.5 * s.idle && top > 0.85 * s.redline;
    (format!("finite {:<5} top {:>5.0} (redline {:>5.0})  lowest after lift {:>5.0}  settled {:>5.0} (idle {:>4.0})  boost {:+6.1} to {:+6.1} kPa", finite, top, s.redline, lo_after, settled, s.idle, min_boost / 1e3, peak_boost / 1e3), ok, trace)
}

pub fn run(args: &[String]) {
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let generic_bar = flag("--generic").map(|x| x.parse::<f64>().expect("--generic <bar>"));
    let pos: Vec<&String> = {
        let mut v = Vec::new();
        let mut i = 0;
        while i < args.len() {
            if args[i].starts_with("--") {
                i += 2;
            } else {
                v.push(&args[i]);
                i += 1;
            }
        }
        v
    };
    let what = pos.first().map_or("", |s| s.as_str());
    match what {
        "rig" => rig(),
        "curve" => {
            let s = engine_arg(pos[1], generic_bar);
            let q = quality(&flag("--quality").unwrap_or_else(|| "live96".into()));
            let rpms: Vec<f64> = match flag("--rpms") {
                Some(l) => l.split(',').map(|x| x.parse().unwrap()).collect(),
                None => (2..=8).map(|i| (s.redline * i as f64 / 8.0 / 100.0).round() * 100.0).collect(),
            };
            curve(&s, &q, &rpms);
        }
        "whine" => {
            let s = engine_arg(pos[1], generic_bar);
            let qn = flag("--quality").unwrap_or_else(|| "live96+W".into());
            let q = quality(&qn);
            let secs: f64 = flag("--secs").map_or(1.0, |x| x.parse().unwrap());
            let rpms: Vec<f64> = match flag("--rpms") {
                Some(l) => l.split(',').map(|x| x.parse().unwrap()).collect(),
                None => vec![0.4 * s.redline, 0.8 * s.redline],
            };
            let mut ok = true;
            for &rpm in &rpms {
                ok &= whine(&s, &q, rpm.round(), secs);
            }
            // the line must follow the drive ratio, not the engine
            let mut s2 = s.clone();
            s2.boost.as_mut().unwrap().drive_ratio *= 1.07;
            println!("same engine with the drive ratio 7% higher:");
            ok &= whine(&s2, &q, rpms[0].round(), secs);
            println!("{} whine line at pockets x rotor speed in the intake and the plenum (within 0.5%, and stronger than any engine order near it)", if ok { "PASS" } else { "FAIL" });
        }
        "stability" => {
            let s = engine_arg(pos[1], generic_bar);
            let levels: Vec<String> = match flag("--qualities") {
                Some(l) => l.split(',').map(|x| x.to_string()).collect(),
                None => ["live48+W", "live96+W", "live48", "live96", "draft", "final"].iter().map(|x| x.to_string()).collect(),
            };
            let mut all_ok = true;
            for qn in &levels {
                let q = quality(if qn == "draft" { "" } else { qn });
                let (line, ok, _) = rev_run(&s, &q, 7);
                println!("{} {:<9} {}", if ok { "PASS" } else { "FAIL" }, qn, line);
                all_ok &= ok;
            }
            if !all_ok {
                std::process::exit(1);
            }
        }
        "trace" => {
            // trace <engine>: the rev run as CSV (t, rpm, boost kPa, bypass opening) every 10 ms
            let s = engine_arg(pos[1], generic_bar);
            let q = quality(&flag("--quality").unwrap_or_else(|| "live48+W".into()));
            let (line, _, tr) = rev_run(&s, &q, 7);
            println!("t,rpm,boost_kpa,bypass");
            for r in tr {
                println!("{:.2},{:.0},{:.2},{:.3}", r[0], r[1], r[2], r[3]);
            }
            eprintln!("{line}");
        }
        "all" => {
            // every library engine that can take a blower, with the generic test blower
            let q = quality(&flag("--quality").unwrap_or_else(|| "live48+W".into()));
            let bar = generic_bar.unwrap_or(0.5);
            let (mut n_ok, mut n) = (0, 0);
            for id in spec::names() {
                let base = load_spec(id);
                if base.itb || base.is_two_stroke() || !base.chamber.is_empty() {
                    continue;
                }
                let s = if base.boost.is_some() { base.clone() } else { generic(&base, bar) };
                let (line, ok, _) = rev_run(&s, &q, 7);
                n += 1;
                n_ok += ok as usize;
                println!("{} {:<28} {}", if ok { "PASS" } else { "FAIL" }, id, line);
            }
            println!("{n_ok} of {n} plenum engines pass with a blower (generic at {bar} bar where the engine has none)");
        }
        _ => eprintln!("usage: engsim boost rig | curve <engine> [--rpms a,b] [--quality q] | whine <engine> [--rpms a,b] [--secs s] [--quality q] | stability <engine> [--qualities a,b] | trace <engine> | all [--quality q]   (add --generic <bar> to put the generic test blower on an engine that has none; ENGSIM_BOOST_LAYOUT=after gives it the throttle after the blower, an intercooler and a charge pipe, =screw a twin-screw)"),
    }
}
