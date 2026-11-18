//! Checks for the turbochargers: `engsim turbo ...` (usage in `run`). Numeric only; nobody has
//! listened. The generic turbo here is a test fixture: never a library engine, never in the app.

use super::*;
use engsim::pipe::*;
use engsim::turbo::*;

/// A copy of `s` with a generic turbocharger sized by the rules in `BoostSpec::finish_turbo`.
pub fn generic(s: &spec::EngineSpec, boost_bar: f64, extra: &serde_json::Value) -> spec::EngineSpec {
    let mut v = s.clone();
    let mut j = serde_json::json!({"kind": "turbo", "induction": "test fixture", "boost_bar": boost_bar, "intercooler": {}, "turbo": {}});
    if let Some(m) = extra.as_object() {
        for (k, val) in m {
            if k == "intercooler" && val.is_null() {
                j.as_object_mut().unwrap().remove("intercooler");
            } else if k == "boost" {
                continue;
            } else {
                j["turbo"][k] = val.clone();
            }
        }
    }
    v.boost = Some(serde_json::from_value(j).unwrap());
    v.finish().expect("generic turbo")
}

fn engine_arg(name: &str, bar: Option<f64>, extra: &serde_json::Value) -> spec::EngineSpec {
    let s = load_spec(name);
    match bar {
        Some(b) => generic(&s, b, extra),
        None => {
            if s.boost.as_ref().and_then(|b| b.turbo.as_ref()).is_none() {
                eprintln!("{name} has no turbocharger; add --generic <bar> for the generic test turbo");
                std::process::exit(2);
            }
            // --set on a real engine changes its turbo fields (for trials; other derived fields are not re-derived)
            match extra.as_object() {
                Some(m) if !m.is_empty() => {
                    let mut v = serde_json::to_value(&s).unwrap();
                    for (k, val) in m {
                        if v["boost"]["turbo"].get(k).is_some() {
                            v["boost"]["turbo"][k] = val.clone();
                        } else {
                            // an engine field (optional ones are absent until set)
                            v[k] = val.clone();
                        }
                    }
                    serde_json::from_value::<spec::EngineSpec>(v).unwrap().finish().unwrap()
                }
                _ => s,
            }
        }
    }
}

pub struct TPoint {
    pub torque: f64,
    pub kw: f64,
    pub boost_kpa: f64,
    pub shaft_krpm: f64,
    pub pr: f64,
    pub margin: f64,
    pub wg: f64,
    pub p_t: f64,
    pub p_c: f64,
    /// (turbine work - compressor work - bearing loss - change of shaft energy) / turbine work
    pub e_err: f64,
    /// (compressor mass - cylinder air - blow-off) / compressor mass
    pub m_err: f64,
    /// (compressor mass - throttle mass - blow-off - change held between them) / compressor mass
    pub m_err_thr: f64,
    /// standard deviation of plenum pressure over the last cycles, kPa (steadiness)
    pub boost_sd: f64,
    /// spread (max - min) of the boost averaged over each engine cycle, kPa: slow oscillation, not pulsation
    pub boost_swing: f64,
    /// mean turbine inlet and outlet pressure, kPa gauge
    pub px: f64,
    /// mean charge temperature at the throttle and at the compressor outlet, K
    pub t_charge: f64,
    pub t_comp: f64,
    pub py: f64,
    /// per turbo (parallel twins): mean shaft speed (krpm), wastegate opening, surge margin, compressor flow (kg/s)
    pub each: Vec<[f64; 4]>,
}

/// Steady full throttle on the dyno after `warm` seconds (the shaft has to spool).
pub fn tpoint(s: &spec::EngineSpec, q: &Quality, rpm: f64, warm: f64, throttle: f64) -> TPoint {
    let mut sys = System::build(s, q, 12345);
    sys.mode = Mode::Dyno;
    sys.omega = rpm * PI / 30.0;
    sys.throttle_cmd = throttle;
    for _ in 0..(warm * q.rate) as usize {
        sys.advance();
    }
    sys.reset_stats();
    // energy and mass balances are summed over the turbos (parallel twins); the per-turbo figures
    // reported are the first turbo's
    let sums = |sys: &System| -> [f64; 6] {
        let mut a = [0.0; 6];
        for t in &sys.turbos {
            for (x, v) in a.iter_mut().zip([t.w_t, t.w_c, t.w_b, 0.5 * t.inertia * t.omega * t.omega, t.m_c, t.m_bov]) {
                *x += v;
            }
        }
        a
    };
    let [wt0, wc0, wb0, ek0, mc0, mb0] = sums(&sys);
    let per_cycle = (120.0 / rpm * q.rate) as usize;
    let n = per_cycle * 12;
    let (mut pw, mut pw2, mut sh, mut pr, mut mg, mut wg, mut px, mut py, mut tch, mut tco) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    // the gas held in the intake between compressor and cylinders changes a little; count it
    let held = |sys: &System| -> f64 {
        let mut m = 0.0;
        for (i, v) in sys.vols.iter().enumerate() {
            if !sys.turbos.iter().any(|t| i == t.u || i == t.y || t.scrolls.iter().any(|sc| sc.x == i)) && !sys.collectors.contains(&i) && !v.name.starts_with("merge") && !v.name.starts_with("bov") {
                m += v.m;
            }
        }
        for p in sys.pipes.iter().filter(|p| p.kind == "charge" || p.kind == "runner") {
            m += p.mass();
        }
        m
    };
    let h0 = held(&sys);
    // the charge path between compressor and throttle only
    let held_charge = |sys: &System| -> f64 {
        let t = &sys.turbos[0];
        let mut m = 0.0;
        for v in sys.vols.iter() {
            let base = v.name.split("_t").next().unwrap_or("");
            if ["comp_out", "ic_in", "intercooler", "throttle_in"].contains(&v.name.as_str()) || (v.name.contains("_t") && ["comp_out", "ic_in", "intercooler"].contains(&base)) {
                m += v.m;
            }
        }
        for p in sys.pipes.iter().filter(|p| p.kind == "charge") {
            m += p.mass();
        }
        let _ = t;
        m
    };
    let hc0 = held_charge(&sys);
    let thr = sys.orifs.iter().position(|o| o.throttle).unwrap();
    let mut m_thr = 0.0;
    let mut cyc_means = Vec::new();
    let mut cyc_acc = 0.0;
    let mut each = vec![[0.0f64; 4]; sys.turbos.len()];
    for k in 0..n {
        sys.advance();
        for (e, t) in each.iter_mut().zip(&sys.turbos) {
            e[0] += t.omega;
            e[1] += t.wg.as_ref().map_or(0.0, |w| w.x);
            e[2] += t.surge_margin;
            e[3] += t.mdot_c;
        }
        cyc_acc += sys.vols[sys.plenum.unwrap()].p();
        if (k + 1) % per_cycle == 0 {
            cyc_means.push(cyc_acc / per_cycle as f64);
            cyc_acc = 0.0;
        }
        m_thr += sys.orifs[thr].mdot / q.rate;
        let pl = sys.vols[sys.plenum.unwrap()].p();
        pw += pl;
        pw2 += pl * pl;
        let t = &sys.turbos[0];
        sh += t.omega;
        pr += t.pr_c;
        mg += t.surge_margin;
        wg += t.wg.as_ref().map_or(0.0, |w| w.x);
        px += sys.vols[t.scrolls[0].x].p();
        py += sys.vols[t.y].p();
        tch += sys.vols[t.bov.as_ref().map_or(t.d, |b| b.from)].t();
        tco += sys.vols[t.d].t();
    }
    let h1 = held(&sys);
    let hc1 = held_charge(&sys);
    let nf = n as f64;
    let st = sys.stats;
    let torque = (st.work - st.fric_work - st.drive_work) / st.angle;
    let secs = nf / q.rate;
    let [wt1, wc1, wb1, ek1, mc1, mb1] = sums(&sys);
    let dwt = wt1 - wt0;
    let e_err = (dwt - (wc1 - wc0) - (wb1 - wb0) - (ek1 - ek0)) / dwt.max(1e-9);
    let mc = mc1 - mc0;
    let m_err = (mc - st.air - (mb1 - mb0) - (h1 - h0)) / mc.max(1e-12);
    let m_err_thr = (mc - m_thr - (mb1 - mb0) - (hc1 - hc0)) / mc.max(1e-12);
    let mean = pw / nf;
    TPoint {
        torque,
        kw: torque * rpm * PI / 30.0 / 1000.0,
        boost_kpa: (mean - P_ATM) / 1e3,
        shaft_krpm: sh / nf * 30.0 / PI / 1e3,
        pr: pr / nf,
        margin: mg / nf,
        wg: wg / nf,
        p_t: dwt / secs / 1e3,
        p_c: (wc1 - wc0) / secs / 1e3,
        e_err,
        m_err,
        m_err_thr,
        boost_sd: ((pw2 / nf - mean * mean).max(0.0)).sqrt() / 1e3,
        boost_swing: (cyc_means.iter().cloned().fold(f64::MIN, f64::max) - cyc_means.iter().cloned().fold(f64::MAX, f64::min)) / 1e3,
        px: (px / nf - P_ATM) / 1e3,
        t_charge: tch / nf,
        t_comp: tco / nf,
        py: (py / nf - P_ATM) / 1e3,
        each: each.iter().map(|e| [e[0] / nf * 30.0 / PI / 1e3, e[1] / nf, e[2] / nf, e[3] / nf]).collect(),
    }
}

fn curve(s: &spec::EngineSpec, q: &Quality, rpms: &[f64], warm: f64) -> bool {
    let b = s.boost.as_ref().unwrap();
    let t = b.turbo.as_ref().unwrap();
    println!("{} — {}: turbo, compressor {:.1} mm, turbine area {:.2} cm^2 ({:.1} mm wheel), inertia {:.2e} kg m^2, wastegate cracks at {:.2} bar, blow-off {}, {}", s.name, s.label, t.d2 * 1e3, t.turbine_area * 1e4, t.turbine_d * 1e3, t.inertia, t.wg_crack_bar, t.bov, if b.intercooler.is_some() { "intercooled" } else { "no intercooler" });
    println!("{:>6} {:>7} {:>6} {:>9} {:>5} {:>9} {:>6} {:>7} {:>6} {:>8} {:>8} {:>9} {:>9} {:>8}", "rpm", "N·m", "kW", "boost kPa", "psi", "swing", "shaft", "PR", "surge", "wg open", "turb kW", "energy", "mass", "NA N·m");
    println!("       (turbine inlet / outlet, kPa gauge, follow each row)");
    let na = {
        let mut v = s.clone();
        v.boost = None;
        v
    };
    let mut ok = true;
    for &rpm in rpms {
        let p = tpoint(s, q, rpm, warm, 1.0);
        let pn = point_na(&na, q, rpm);
        println!("{:>6.0} {:>7.1} {:>6.1} {:>9.1} {:>5.1} {:>9.2} {:>5.0}k {:>7.3} {:>+6.2} {:>8.2} {:>8.1} {:>+8.2}% {:>+8.2}% {:>8.1}", rpm, p.torque, p.kw, p.boost_kpa, p.boost_kpa / 6.894757, p.boost_swing, p.shaft_krpm, p.pr, p.margin, p.wg, p.p_t, 100.0 * p.e_err, 100.0 * p.m_err, pn);
        println!("       turbine in {:+.1} / out {:+.1} kPa, compressor {:.1} kW; charge {:.0} K after the compressor, {:.0} K at the throttle; mass: compressor against throttle + blow-off {:+.3}%", p.px, p.py, p.p_c, p.t_comp, p.t_charge, 100.0 * p.m_err_thr);
        if p.each.len() > 1 {
            let parts: Vec<String> = p.each.iter().enumerate().map(|(k, e)| format!("turbo {}: shaft {:.0}k, wastegate {:.2}, surge {:+.2}, {:.3} kg/s", k + 1, e[0], e[1], e[2], e[3])).collect();
            println!("       {}", parts.join("; "));
        }
        ok &= p.boost_swing < 0.05 * p.boost_kpa.abs().max(20.0) && p.e_err.abs() < 0.01 && p.m_err_thr.abs() < 0.005 && p.torque.is_finite();
    }
    println!("{} settled (cycle-mean boost varies by under 5% over 12 cycles), energy balance within 1% (turbine = compressor + bearings + change of shaft energy) and mass balance: compressor = throttle + blow-off (+ change held between them) within 0.5%, (the 'mass' column, compressor against the cylinders' trapped air, is shown but not tested: the engine model counts fresh charge at intake closing, so charge that short-circuits to the exhaust in valve overlap is not in it)", if ok { "PASS" } else { "FAIL" });
    ok
}

fn point_na(s: &spec::EngineSpec, q: &Quality, rpm: f64) -> f64 {
    let mut sys = System::build(s, q, 12345);
    sys.mode = Mode::Dyno;
    sys.omega = rpm * PI / 30.0;
    sys.throttle_cmd = 1.0;
    let per_cycle = (120.0 / rpm * q.rate) as usize;
    for _ in 0..per_cycle * 10 {
        sys.advance();
    }
    sys.reset_stats();
    for _ in 0..per_cycle * 6 {
        sys.advance();
    }
    let st = sys.stats;
    (st.work - st.fric_work - st.drive_work) / st.angle
}

/// T1: the compressor alone between a large inlet volume at ambient and a 2 L outlet volume that
/// drains through an orifice, at fixed shaft speeds. The steady points must sit on the speed line
/// the map defines; below the surge flow the system oscillates (the Greitzer duct against the volume).
fn rig_compressor(t: &spec::TurboSpec) -> bool {
    println!("T1 compressor rig: wheel {:.1} mm, map peak head {:.2} at the surge flow coefficient, shut-off {:.2}; outlet 2 L draining through an orifice, 96 kHz", t.d2 * 1e3, t.psi_peak, t.psi_shut);
    println!("{:>7} {:>9} {:>8} {:>8} {:>9} {:>9} {:>11}", "tip m/s", "orifice", "phi", "PR sim", "PR map", "error", "osc PR p-p");
    let rate = 96_000.0;
    let dt = 1.0 / rate;
    let mut worst: f64 = 0.0;
    let mut surge_seen = false;
    let mut stable_right = true;
    for &tip in &[200.0, 300.0, 400.0] {
        for &a_cm2 in &[30.0, 15.0, 8.0, 4.0, 2.0, 1.0] {
            let mut vols = vec![Volume::new("u", 10.0, T_ATM), Volume::new("d", 2.0e-3, T_ATM)];
            let mut tb = Turbo::new(0, 1, Vec::new(), 1, t, 1);
            tb.whistle = 0.0;
            tb.omega = 2.0 * tip / t.d2;
            let mut prs = Vec::new();
            let mut flows = Vec::new();
            for i in 0..(0.6 * rate) as usize {
                let om = tb.omega;
                tb.step(&mut vols, dt);
                tb.omega = om; // held speed
                let _ = boostlib_orifice(&mut vols, 1, a_cm2 * 1e-4, dt);
                for v in vols.iter_mut() {
                    v.apply();
                }
                vols[0].set(P_ATM, T_ATM);
                if i as f64 > 0.4 * rate {
                    prs.push(vols[1].p() / P_ATM);
                    flows.push(tb.mdot_c);
                }
            }
            let mean_pr = prs.iter().sum::<f64>() / prs.len() as f64;
            let pp = prs.iter().cloned().fold(f64::MIN, f64::max) - prs.iter().cloned().fold(f64::MAX, f64::min);
            let mdot = flows.iter().sum::<f64>() / flows.len() as f64;
            let phi = mdot / (RHO_ATM * tip * t.d2 * t.d2) / tb.scale(tip);
            let pr_map = tb.pr_at(mdot, RHO_ATM, T_ATM);
            // the outlet sits a little above the map by the duct's own nothing: compare directly
            let err = mean_pr / pr_map - 1.0;
            let surging = pp > 0.02;
            println!("{:>7.0} {:>6.1}cm2 {:>8.4} {:>8.4} {:>9.4} {:>+8.2}% {:>11.4}{}", tip, a_cm2, phi, mean_pr, pr_map, 100.0 * err, pp, if surging { "  oscillating" } else { "" });
            if phi > tb.surge_phi() * 1.05 && !surging {
                worst = worst.max(err.abs());
            }
            if phi > tb.surge_phi() * 1.1 && surging {
                stable_right = false;
            }
            if phi < tb.surge_phi() && surging {
                surge_seen = true;
            }
        }
    }
    let ok = worst < 0.01 && stable_right && surge_seen;
    println!("{} steady points right of surge on the map within 1% (worst {:.2}%), steady right of the surge line: {}, oscillation left of it: {}", if ok { "PASS" } else { "FAIL" }, 100.0 * worst, stable_right, surge_seen);
    ok
}

/// Orifice from a volume to the air, as the rig's throttle.
fn boostlib_orifice(vols: &mut [Volume], i: usize, area: f64, dt: f64) -> f64 {
    let (p, t) = (vols[i].p(), vols[i].t());
    if p <= P_ATM {
        return 0.0;
    }
    let mut dm = orifice_mdot(0.8 * area, p, t, P_ATM) * dt;
    let dm_eq = (p - P_ATM) / (GAMMA * R_GAS * t / vols[i].v);
    dm = dm.min(0.5 * dm_eq);
    vols[i].m -= dm;
    vols[i].e -= dm * CP * t;
    dm
}

/// T2: the turbine alone between a large volume at 1000 K and the air side at 1 bar, at fixed
/// shaft speeds: flow must be the nozzle's, and efficiency against blade speed ratio the parabola.
fn rig_turbine(t: &spec::TurboSpec) -> bool {
    println!("\nT2 turbine rig: nozzle {:.2} cm^2, wheel {:.1} mm, peak efficiency {:.2} at blade speed ratio 0.7", t.turbine_area * 1e4, t.turbine_d * 1e3, t.eta_t);
    println!("{:>5} {:>8} {:>10} {:>10} {:>7} {:>7} {:>9}", "ER", "shaft", "flow kg/s", "nozzle", "BSR", "eta", "expected");
    let dt = 1.0 / 96_000.0;
    let mut worst: f64 = 0.0;
    for &er in &[1.3, 1.8, 2.5] {
        for &krpm in &[40.0, 90.0, 140.0] {
            let mut vols = vec![Volume::new("u", 1e-3, T_ATM), Volume::new("d", 1e-3, T_ATM), Volume::new("x", 10.0, 1000.0), Volume::new("y", 10.0, 700.0)];
            vols[2].set(P_ATM * er, 1000.0);
            vols[3].set(P_ATM, 700.0);
            let scr = vec![Scroll { x: 2, area: t.turbine_area, mdot: 0.0 }];
            let mut tb = Turbo::new(0, 1, scr, 3, t, 1);
            tb.whistle = 0.0;
            tb.c_bear = 0.0;
            let om = krpm * 1000.0 * PI / 30.0;
            tb.omega = om;
            let (w0, m0) = (tb.w_t, tb.m_t);
            let n = 4800;
            for _ in 0..n {
                tb.step(&mut vols, dt);
                tb.omega = om;
                tb.mdot_c = 0.0;
                for v in vols.iter_mut() {
                    v.apply();
                }
                vols[2].set(P_ATM * er, 1000.0);
                vols[3].set(P_ATM, 700.0);
                vols[0].set(P_ATM, T_ATM);
                vols[1].set(P_ATM, T_ATM);
            }
            let m = (tb.m_t - m0) / (n as f64 * dt);
            let nozzle = orifice_mdot(t.turbine_area, P_ATM * er, 1000.0, P_ATM);
            let dhs = CP * 1000.0 * (1.0 - pow_gm1_g(1.0 / er));
            let eta = (tb.w_t - w0) / (tb.m_t - m0) / dhs;
            let bsr = 0.5 * om * t.turbine_d / (2.0 * dhs).sqrt();
            let r = (bsr - 0.7) / t.bsr_width;
            let expect = (t.eta_t * (1.0 - r * r)).max(0.0);
            println!("{:>5.1} {:>6.0}k {:>10.4} {:>10.4} {:>7.3} {:>7.3} {:>9.3}", er, krpm, m, nozzle, bsr, eta, expect);
            worst = worst.max((m / nozzle - 1.0).abs()).max((eta - expect).abs());
        }
    }
    let ok = worst < 0.01;
    println!("{} turbine flow on the nozzle curve and efficiency on the blade-speed-ratio parabola (worst error {:.3})", if ok { "PASS" } else { "FAIL" }, worst);
    ok
}

/// T5: transmission loss of the turbine and the compressor as two-ports. A pulse travels down a
/// long pipe into the element's inlet volume; the pulse leaving through a long pipe on the other
/// side is compared with the same rig without the element (the two volumes joined by a wide open
/// orifice). Pipes long enough that the first arrival is clean.
fn rig_tl(t: &spec::TurboSpec) {
    println!("\nT5 transmission loss (insertion loss of the element, first arrival, mean flow through it):");
    let q = quality("live96");
    let dt = 1.0 / q.rate;
    for (name, turbine) in [("turbine", true), ("compressor", false)] {
        let run = |with: bool, pulsed: bool| -> (Vec<f64>, Vec<f64>, f64) {
            let temp = if turbine { 900.0 } else { T_ATM };
            let d = 0.05;
            let mut a = Pipe::new("a", "secondary", &[Sec { len: 6.0, d0: d, d1: d, fric: 0.0 }], q.dx("secondary"), End::Volume(0), End::Volume(1), temp, temp);
            let mut b = Pipe::new("b", "secondary", &[Sec { len: 6.0, d0: d, d1: d, fric: 0.0 }], q.dx("secondary"), End::Volume(2), End::Volume(3), temp, temp);
            let p_in = if turbine { 1.6 * P_ATM } else { P_ATM };
            // the open-link reference runs without mean flow; the element runs with its own
            let p_out = if !with { p_in } else if turbine { P_ATM } else { 1.4 * P_ATM };
            let c = (GAMMA * R_GAS * temp).sqrt();
            let mut vols = vec![Volume::new("src", 1.0, temp), Volume::new("x", 4e-4, temp), Volume::new("y", 4e-4, temp), Volume::new("sink", 1.0, temp)];
            vols[0].set(p_in, temp);
            vols[3].set(p_out, temp);
            vols[2].set(p_out, temp);
            fill_pipe(&mut a, p_in, temp);
            fill_pipe(&mut b, p_out, temp);
            let scr = vec![Scroll { x: 1, area: t.turbine_area, mdot: 0.0 }];
            let mut tb = if turbine { Turbo::new(4, 5, scr, 2, t, 1) } else { Turbo::new(1, 2, Vec::new(), 2, t, 1) };
            tb.whistle = 0.0;
            tb.wg = None;
            tb.bov = None;
            let om = if turbine { 2.0 * 0.7 * (2.0 * CP * temp * (1.0 - pow_gm1_g(1.0 / 1.6))).sqrt() / t.turbine_d } else { 2.0 * 330.0 / t.d2 };
            let (mut sa, mut sb) = (Vec::new(), Vec::new());
            let n = (0.12 * q.rate) as usize;
            let mut dummy = vec![Volume::new("u", 1.0, T_ATM), Volume::new("d", 1.0, T_ATM)];
            for i in 0..n {
                let time = i as f64 * dt;
                // a 0.4 ms raised-cosine pulse in the source pressure at 50 ms, after the mean flow has settled
                let pulse = if pulsed && (0.05..0.0504).contains(&time) { 0.5 - 0.5 * (2.0 * PI * (time - 0.05) / 0.0004).cos() } else { 0.0 };
                vols[0].set(p_in * (1.0 + 0.05 * pulse), temp);
                vols[3].set(p_out, temp);
                let bl = Bc::Res { p0: vols[0].p(), rho0: vols[0].rho() };
                let br = Bc::Res { p0: vols[1].p(), rho0: vols[1].rho() };
                a.step(dt, bl, br);
                vols[1].m += a.out_r.mdot * dt;
                vols[1].e += a.out_r.edot * dt;
                let bl2 = Bc::Res { p0: vols[2].p(), rho0: vols[2].rho() };
                let br2 = Bc::Res { p0: vols[3].p(), rho0: vols[3].rho() };
                b.step(dt, bl2, br2);
                vols[2].m += b.out_l.mdot * dt;
                vols[2].e += b.out_l.edot * dt;
                if with {
                    if turbine {
                        // the turbine moves gas x -> y; its compressor side runs in a separate dummy pair
                        let mut all: Vec<Volume> = Vec::new();
                        std::mem::swap(&mut all, &mut vols);
                        all.push(std::mem::replace(&mut dummy[0], Volume::new("u", 1.0, T_ATM)));
                        all.push(std::mem::replace(&mut dummy[1], Volume::new("d", 1.0, T_ATM)));
                        tb.omega = om;
                        tb.mdot_c = 0.0;
                        tb.step(&mut all, dt);
                        dummy[1] = all.pop().unwrap();
                        dummy[0] = all.pop().unwrap();
                        vols = all;
                    } else {
                        tb.omega = om;
                        tb.step(&mut vols, dt);
                    }
                } else {
                    boost_link(&mut vols, 1, 2, 2.0e-3, dt);
                }
                for v in vols.iter_mut() {
                    v.apply();
                }
                // probes 1.5 m into each pipe
                let pa = a.prim(((1.5 / a.len) * a.n as f64) as usize).p;
                let pb = b.prim(((1.5 / b.len) * b.n as f64) as usize).p;
                if !pa.is_finite() || !pb.is_finite() {
                    eprintln!("  ({} rig went non-finite at {:.4} s, with element: {}; x {:.0} Pa, y {:.0} Pa, compressor flow {:.4})", if turbine { "turbine" } else { "compressor" }, time, with, vols[1].p(), vols[2].p(), tb.mdot_c);
                    return (vec![f64::NAN], vec![f64::NAN], c);
                }
                sa.push(pa);
                sb.push(pb);
            }
            (sa, sb, c)
        };
        // the pulse's own signal: the same deterministic run with and without it, subtracted, so the
        // start-up transient cancels; first arrivals only (before the element's reflection returns to
        // the inlet probe, and before the far end's reflection returns to the outlet probe)
        let peaks = |with: bool| -> (f64, f64) {
            let (a1, b1, c) = run(with, true);
            let (a0, b0, _) = run(with, false);
            let (mut pi, mut po) = (0.0f64, 0.0f64);
            for k in 0..a1.len().min(a0.len()) {
                let time = k as f64 * dt;
                if time > 0.05 && time < 0.05 + 10.0 / c {
                    pi = pi.max(a1[k] - a0[k]);
                }
                if time > 0.05 && time < 0.05 + 15.0 / c {
                    po = po.max(b1[k] - b0[k]);
                }
            }
            (pi, po)
        };
        let (i0, o0) = peaks(false);
        let (i1, o1) = peaks(true);
        let il = 20.0 * ((o0 / i0) / (o1 / i1)).log10();
        // the pulse is about 0.4 ms long: its energy is mostly below 2.5 kHz; He = k a at 1 kHz in a 50 mm duct
        let c = if turbine { (GAMMA * R_GAS * 900.0).sqrt() } else { (GAMMA * R_GAS * T_ATM).sqrt() };
        let _ = (i0, i1);
        println!("  {:<10} insertion loss {:.1} dB (transmitted over incident peak: open link {:.3}, with the {} {:.3}); He at 1 kHz = {:.2}. Tiikoja's measured units: about 1 to 15 dB below He 0.2", name, il, o0 / i0, name, o1 / i1, 2.0 * PI * 1000.0 / c * 0.025);
    }
}

/// Set a grid pipe to gas at rest at pressure p and temperature t.
fn fill_pipe(p: &mut Pipe, pr: f64, t: f64) {
    for i in 0..p.n {
        p.rho[i] = pr / (R_GAS * t);
        p.mom[i] = 0.0;
        p.ene[i] = pr / GM1;
    }
}

/// Wide open link between two volumes (the rig without an element).
fn boost_link(vols: &mut [Volume], a: usize, b: usize, area: f64, dt: f64) {
    let _ = engsim::boost::orifice_between(vols, a, b, area, dt, None);
}

/// T6/T7 and stage-4 runs on the dyno or in free running.
fn spool(s: &spec::EngineSpec, q: &Quality, rpm: f64, seed: u64) -> (f64, f64, f64) {
    let mut sys = System::build(s, q, seed);
    sys.mode = Mode::Dyno;
    sys.omega = rpm * PI / 30.0;
    sys.throttle_cmd = 0.15;
    for _ in 0..(1.5 * q.rate) as usize {
        sys.advance();
    }
    sys.throttle_cmd = 1.0;
    let pl = sys.plenum.unwrap();
    // boost averaged over one engine cycle, so pulsation is not counted as overshoot
    let per_cycle = (120.0 / rpm * q.rate) as usize;
    let mut ring = vec![P_ATM; per_cycle];
    let mut acc = P_ATM * per_cycle as f64;
    let mut trace = Vec::new();
    for i in 0..(5.0 * q.rate) as usize {
        sys.advance();
        let p = sys.vols[pl].p();
        acc += p - ring[i % per_cycle];
        ring[i % per_cycle] = p;
        if i % 48 == 0 && i >= per_cycle {
            trace.push((i as f64 / q.rate, acc / per_cycle as f64 - P_ATM));
        }
    }
    let target = trace.iter().rev().take(200).map(|x| x.1).sum::<f64>() / 200.0;
    let t90 = trace.iter().find(|x| x.1 >= 0.9 * target).map_or(f64::NAN, |x| x.0);
    let overshoot = trace.iter().map(|x| x.1).fold(f64::MIN, f64::max) / target - 1.0;
    (t90, target / 1e3, overshoot)
}

/// Free running: idle, full throttle into the limiter, lift with the blow-off valve, back to idle.
pub fn rev_run(s: &spec::EngineSpec, q: &Quality, seed: u64, hold: f64) -> (String, bool, Vec<[f64; 6]>) {
    rev_run_with(s, q, seed, hold, false)
}

/// The same, optionally with every fueling setting at its harshest (rich mixture, fueled overrun,
/// spark-cut limiter, lift-off pops at 1): afterfire and pops through the turbine.
pub fn rev_run_with(s: &spec::EngineSpec, q: &Quality, seed: u64, hold: f64, harsh: bool) -> (String, bool, Vec<[f64; 6]>) {
    let mut sys = System::build(s, q, seed);
    sys.mode = Mode::Free;
    if harsh {
        sys.phi = 1.7;
        sys.overrun_fuel = 1.0;
        sys.limiter_kind = Limiter::SparkCut;
        sys.liftoff = 1.0;
    }
    let (mut finite, mut top, mut lo_after, mut sum, mut n, mut peak_boost) = (true, 0.0f64, f64::MAX, 0.0, 0.0, f64::MIN);
    let mut trace = Vec::new();
    let total = 4.0 + hold + 4.0;
    for i in 0..(total * q.rate) as usize {
        let t = i as f64 / q.rate;
        sys.throttle_cmd = if (1.0..1.0 + hold).contains(&t) { 1.0 } else { 0.0 };
        let o = sys.advance();
        finite &= o.iter().all(|v| v.is_finite());
        let r = sys.rpm();
        let bst = sys.vols[sys.plenum.unwrap()].p() - P_ATM;
        if t < 1.1 + hold {
            top = top.max(r);
        } else {
            lo_after = lo_after.min(r);
        }
        peak_boost = peak_boost.max(bst);
        if t > total - 1.5 {
            sum += r;
            n += 1.0;
        }
        if i % (q.rate as usize / 200) == 0 {
            let tb = &sys.turbos[0];
            trace.push([t, r, bst / 1e3, tb.shaft_rpm(), tb.surge_margin, tb.bov.as_ref().map_or(0.0, |b| b.x)]);
        }
    }
    let settled = sum / n;
    let ok = finite && (settled / s.idle - 1.0).abs() < if harsh { 0.4 } else { 0.25 } && lo_after > if harsh { 0.4 } else { 0.5 } * s.idle && top > 0.85 * s.redline;
    let pops: u64 = sys.af.iter().map(|a| a.events).sum();
    (format!("afterfire events {:>5}  finite {:<5} top {:>5.0} (redline {:>5.0})  lowest after lift {:>5.0}  settled {:>5.0} (idle {:>4.0})  peak boost {:+6.1} kPa", pops, finite, top, s.redline, lo_after, settled, s.idle, peak_boost / 1e3), ok, trace)
}

/// Stage 4: the lift. Steady full throttle on the dyno at `rpm`, then the throttle shuts and the
/// speed is held (as in gear for the first moments of a lift). Returns a report line and pass/fail.
fn lift(s: &spec::EngineSpec, q: &Quality, rpm: f64, bov: &str) -> (String, bool) {
    let mut s2 = s.clone();
    s2.boost.as_mut().unwrap().turbo.as_mut().unwrap().bov = bov.into();
    let s2 = s2.finish().unwrap();
    let mut sys = System::build(&s2, q, 5);
    sys.mode = Mode::Dyno;
    sys.omega = rpm * PI / 30.0;
    sys.throttle_cmd = 1.0;
    for _ in 0..(5.0 * q.rate) as usize {
        sys.advance();
    }
    let t = &sys.turbos[0];
    let td = t.d;
    let bst0 = sys.vols[sys.plenum.unwrap()].p() - P_ATM;
    let shaft0 = t.shaft_rpm();
    // intake microphone level just before the lift
    let (mut pre, mut pre_ex) = (0.0, 0.0);
    let npre = (0.1 * q.rate) as usize;
    for _ in 0..npre {
        let o = sys.advance();
        pre += o[2] * o[2];
        pre_ex += 0.5 * (o[0] * o[0] + o[1] * o[1]);
    }
    pre = (pre / npre as f64).sqrt();
    pre_ex = (pre_ex / npre as f64).sqrt();
    sys.throttle_cmd = 0.0;
    let n = (1.0 * q.rate) as usize;
    let (mut pc, mut mdot, mut intake, mut shaft, mut margin) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
    let mut bov_open: f64 = 0.0;
    let mut finite = true;
    // the vent pipe's own radiation (monopole part) and its exit jet speed
    let vent = sys.pipes.iter().position(|p| p.kind == "bov");
    let (mut q_prev, mut vent_peak, mut vent_e, mut jet_u) = (0.0, 0.0f64, 0.0, 0.0f64);
    for _ in 0..n {
        let o = sys.advance();
        if let Some(vi) = vent {
            let p = &sys.pipes[vi];
            let qv = p.out_r.q;
            let sig = RHO_ATM * (qv - q_prev) * q.rate / (4.0 * PI);
            q_prev = qv;
            vent_peak = vent_peak.max(sig.abs());
            vent_e += sig * sig / q.rate;
            jet_u = jet_u.max(p.prim(p.n - 1).u.abs());
        }
        finite &= o.iter().all(|v| v.is_finite());
        let t = &sys.turbos[0];
        pc.push(sys.vols[td].p());
        mdot.push(t.mdot_c);
        intake.push(o[2]);
        shaft.push(t.shaft_rpm());
        margin.push(t.surge_margin);
        bov_open = bov_open.max(t.bov.as_ref().map_or(0.0, |b| b.x));
    }
    // flow reversals through the compressor (deep surge) after the first 20 ms
    let k0 = (0.02 * q.rate) as usize;
    let rev_at: Vec<usize> = (k0 + 1..n).filter(|&k| mdot[k - 1] >= 0.0 && mdot[k] < 0.0).collect();
    let reversals = rev_at.len();
    let f_rev = if reversals > 2 { (reversals - 1) as f64 / ((rev_at[reversals - 1] - rev_at[0]) as f64 / q.rate) } else { f64::NAN };
    let rev_span = if reversals > 0 { format!("{:.0} to {:.0} ms", 1e3 * rev_at[0] as f64 / q.rate, 1e3 * rev_at[reversals - 1] as f64 / q.rate) } else { "-".into() };
    // oscillation of the compressor outlet pressure: mean-crossing count over 50 to 400 ms
    let (a, b) = ((0.05 * q.rate) as usize, (0.4 * q.rate) as usize);
    let seg: Vec<f64> = pc[a..b].to_vec();
    // remove the slow fall with a 20 ms moving mean
    let w = (0.02 * q.rate) as usize;
    let mut det = Vec::new();
    for k in w..seg.len() - w {
        let m: f64 = seg[k - w..k + w].iter().sum::<f64>() / (2 * w) as f64;
        det.push(seg[k] - m);
    }
    let crossings = (1..det.len()).filter(|&k| det[k - 1] < 0.0 && det[k] >= 0.0).count();
    let f_osc = crossings as f64 / (det.len() as f64 / q.rate);
    let amp = |lo: f64, hi: f64| -> f64 {
        let (i, j) = ((lo * q.rate) as usize, (hi * q.rate) as usize);
        let m = pc[i..j].iter().sum::<f64>() / (j - i) as f64;
        (pc[i..j].iter().map(|v| (v - m) * (v - m)).sum::<f64>() / (j - i) as f64).sqrt()
    };
    let (a1, a2, a3) = (amp(0.05, 0.15), amp(0.15, 0.3), amp(0.5, 0.8));
    // Helmholtz frequency of the compressor duct against the whole charge volume
    let t = &sys.turbos[0];
    let mut v_charge = 0.0;
    for v in &sys.vols {
        if ["comp_out", "ic_in", "intercooler", "throttle_in"].contains(&v.name.as_str()) {
            v_charge += v.v;
        }
    }
    for p in sys.pipes.iter().filter(|p| p.kind == "charge") {
        v_charge += p.vol.iter().sum::<f64>();
    }
    let c = (GAMMA * R_GAS * 320.0).sqrt();
    let f_h_lumped = c / (2.0 * PI) * (t.a_duct / (t.l_duct * v_charge)).sqrt();
    // The charge path is wave pipes, not one volume: the duct's mass resonates against the input
    // reactance of compressor outlet volume -> pipe -> intercooler volumes -> pipe -> throttle inlet
    // (shut throttle), lossless plane waves. First series resonance.
    let rho = (P_ATM + 0.5 * bst0) / (R_GAS * 320.0);
    let comp = |name: &str| sys.vols.iter().filter(|v| v.name == name).map(|v| v.v).sum::<f64>() / (rho * c * c);
    let charge: Vec<(f64, f64)> = sys.pipes.iter().filter(|p| p.kind == "charge").map(|p| (p.len, p.af[0])).collect();
    let reactance = |f: f64| -> f64 {
        let w = 2.0 * PI * f;
        let mut x = -1.0 / (w * comp("throttle_in"));
        let shunts = [comp("ic_in") + comp("intercooler"), comp("comp_out")];
        for (i, &(len, area)) in charge.iter().rev().enumerate() {
            let z0 = rho * c / area;
            let tn = (w / c * len).tan();
            x = z0 * (x + z0 * tn) / (z0 - x * tn);
            let cs = if charge.len() == 1 { shunts[1] } else { shunts[i.min(1)] };
            x = 1.0 / (1.0 / x - w * cs);
        }
        w * rho * t.l_duct / t.a_duct + x
    };
    let mut f_h = f64::NAN;
    let mut prev = reactance(2.0);
    let mut f = 2.0;
    while f < 2000.0 {
        f += 0.25;
        let x = reactance(f);
        if prev < 0.0 && x >= 0.0 && (x - prev).abs() < 1e7 {
            f_h = f;
            break;
        }
        prev = x;
    }
    let peak_after = intake[..(0.15 * q.rate) as usize].iter().map(|v| v.abs()).fold(0.0, f64::max);
    let min_margin = margin[k0..].iter().cloned().fold(f64::MAX, f64::min);
    let line = format!("{:<6} lift at {:.0} rpm from {:+.0} kPa, shaft {:.0}k rpm: blow-off opened {:.2}; flow reversals {} ({}, {:.1} per second); least surge margin {:+.2}; outlet pressure zero crossings {:.1} Hz; first resonance (duct against the charge pipes and volumes, plane waves: {:.1} Hz; lumped Helmholtz with all {:.2} L as one volume: {:.1} Hz), rms {:.2} / {:.2} / {:.2} kPa at 50-150 / 150-300 / 500-800 ms; shaft {:.0}k after 1 s; intake microphone peak in 150 ms {:.2} Pa against {:.2} Pa rms before; vent pipe alone: peak {:.2} Pa at 1 m ({:+.1} dB against the exhaust's rms at full throttle, {:.2} Pa), sound energy {:.2e} Pa^2 s, exit jet up to {:.0} m/s", bov, rpm, bst0 / 1e3, shaft0 / 1e3, bov_open, reversals, rev_span, f_rev, min_margin, f_osc, f_h, v_charge * 1e3, f_h_lumped, a1 / 1e3, a2 / 1e3, a3 / 1e3, shaft[n - 1] / 1e3, peak_after, pre, vent_peak, 20.0 * (vent_peak / pre_ex.max(1e-12)).log10(), pre_ex, vent_e, jet_u);
    let ok = finite
        && match bov {
            "vent" => reversals == 0 && bov_open > 0.5 && vent_peak > 0.0,
            "recirc" => reversals == 0 && bov_open > 0.5,
            // deep-surge cycles (one flow reversal each) at the system's first resonance
            _ => reversals > 2 && (f_rev / f_h - 1.0).abs() < 0.2 && a3 < a1,
        };
    (line, ok)
}

/// Stage 4: the whistle stand-in's partials, from the simulated shaft speed.
fn whistle_check(s: &spec::EngineSpec, q: &Quality, rpms: &[f64], gain: f64) -> bool {
    let mut s2 = s.clone();
    s2.boost.as_mut().unwrap().turbo.as_mut().unwrap().whistle = gain;
    let mut ok = true;
    for &rpm in rpms {
        let mut sys = System::build(&s2, q, 5);
        sys.mode = Mode::Dyno;
        sys.omega = rpm * PI / 30.0;
        sys.throttle_cmd = 1.0;
        for _ in 0..(4.0 * q.rate) as usize {
            sys.advance();
        }
        let n = (0.5 * q.rate) as usize;
        let mut x = Vec::with_capacity(n);
        let mut ex2 = 0.0;
        let mut src = Vec::with_capacity(n);
        let mut sh = 0.0;
        for _ in 0..n {
            let o = sys.advance();
            x.push(o[2]);
            ex2 += 0.5 * (o[0] * o[0] + o[1] * o[1]);
            src.push(sys.turbos[0].wh_q);
            sh += sys.turbos[0].omega;
        }
        let t = &sys.turbos[0];
        let f_rot = sh / n as f64 / (2.0 * PI);
        let mean = x.iter().sum::<f64>() / n as f64;
        let rms = (x.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n as f64).sqrt();
        let mut parts = Vec::new();
        for (name, f) in [("blades", t.blades * f_rot), ("blades+splitters", (t.blades + t.splitters) * f_rot), ("2x that", 2.0 * (t.blades + t.splitters) * f_rot)] {
            let inside = f < 0.45 * q.rate && f < 20_000.0;
            // in the injected source: present below the cap, absent above it; and in the intake
            // microphone, the line against its surroundings (3% either side)
            let s_at = amp_at(&src, q.rate, f);
            let s_all = amp_at(&src, q.rate, t.blades * f_rot).max(1e-30);
            let at = amp_at(&x, q.rate, f);
            let side = amp_at(&x, q.rate, 0.97 * f).max(amp_at(&x, q.rate, 1.03 * f));
            parts.push(format!("{} {:.0} Hz{}: source {:+.1} dB re the blade line, microphone {:+.1} dB over its sides", name, f, if inside { "" } else { " (left out)" }, 20.0 * (s_at / s_all).log10(), 20.0 * (at / side.max(1e-12)).log10()));
            if inside {
                ok &= s_at > 0.1 * s_all && at > side;
            } else {
                ok &= s_at < 0.01 * s_all;
            }
        }
        let ex_rms = (ex2 / n as f64).sqrt();
        let line = amp_at(&x, q.rate, t.blades * f_rot) / std::f64::consts::SQRT_2;
        println!("  whistle line at the intake mouth {:.3} Pa rms: {:+.1} dB against the exhaust's rms ({:.2} Pa), {:+.1} dB against the intake's", line, 20.0 * (line / ex_rms.max(1e-12)).log10(), ex_rms, 20.0 * (line / rms.max(1e-12)).log10());
        println!("  {:.0} rpm, whistle {:.2}: shaft {:.0}k rpm, pressure ratio {:.2}, surge margin {:+.2}; {}; intake rms {:.2} Pa", rpm, gain, f_rot * 60.0 / 1e3, t.pr_c, t.surge_margin, parts.join(", "), rms);
    }
    ok
}

/// Amplitude of the component of `x` at frequency f (Hann window).
fn amp_at(x: &[f64], rate: f64, f: f64) -> f64 {
    let n = x.len();
    let (mut re, mut im, mut ws) = (0.0, 0.0, 0.0);
    for (k, v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (2.0 * PI * k as f64 / (n - 1) as f64).cos();
        let ph = 2.0 * PI * f * k as f64 / rate;
        re += v * w * ph.cos();
        im -= v * w * ph.sin();
        ws += w;
    }
    2.0 * (re * re + im * im).sqrt() / ws
}

pub fn run(args: &[String]) {
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let bar = flag("--generic").map(|x| x.parse::<f64>().expect("--generic <bar>"));
    let extra: serde_json::Value = flag("--set").map(|j| serde_json::from_str(&j).expect("bad --set JSON")).unwrap_or(serde_json::Value::Null);
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
    let qn = flag("--quality").unwrap_or_else(|| "live96".into());
    let q = quality(if qn == "draft" { "" } else { &qn });
    match what {
        "rig" => {
            let s = engine_arg(pos.get(1).map_or("i4", |x| x.as_str()), Some(bar.unwrap_or(0.7)), &extra);
            let t = s.boost.as_ref().unwrap().turbo.clone().unwrap();
            let a = rig_compressor(&t);
            let b = rig_turbine(&t);
            rig_tl(&t);
            if !(a && b) {
                std::process::exit(1);
            }
        }
        "curve" => {
            let s = engine_arg(pos[1], bar, &extra);
            let warm: f64 = flag("--warm").map_or(3.0, |x| x.parse().unwrap());
            let rpms: Vec<f64> = match flag("--rpms") {
                Some(l) => l.split(',').map(|x| x.parse().unwrap()).collect(),
                None => (2..=8).map(|i| (s.redline * i as f64 / 8.0 / 100.0).round() * 100.0).collect(),
            };
            if !curve(&s, &q, &rpms, warm) {
                std::process::exit(1);
            }
        }
        "spool" => {
            let s = engine_arg(pos[1], bar, &extra);
            let rpm: f64 = flag("--rpm").map_or(2000.0, |x| x.parse().unwrap());
            let mut ts = Vec::new();
            for seed in [3u64, 11] {
                let (t90, target, os) = spool(&s, &q, rpm, seed);
                println!("{} full-throttle step at {:.0} rpm on the dyno (seed {}): 90% of the settled {:.1} kPa after {:.2} s, overshoot {:+.1}%", s.name, rpm, seed, target, t90, 100.0 * os);
                ts.push(t90);
            }
            let ok = ts.iter().all(|t| *t < 3.0) && (ts[0] - ts[1]).abs() < 0.05;
            println!("{} spool under 3 s and repeatable (within 0.05 s)", if ok { "PASS" } else { "FAIL" });
        }
        "stability" => {
            let s = engine_arg(pos[1], bar, &extra);
            let levels: Vec<String> = match flag("--qualities") {
                Some(l) => l.split(',').map(|x| x.to_string()).collect(),
                None => ["live48+W", "live96+W", "live48", "live96", "draft", "final"].iter().map(|x| x.to_string()).collect(),
            };
            let mut all = true;
            for l in &levels {
                let q = quality(if l == "draft" { "" } else { l });
                let (line, ok, _) = rev_run(&s, &q, 7, 3.0);
                println!("{} {:<9} {}", if ok { "PASS" } else { "FAIL" }, l, line);
                all &= ok;
            }
            if !all {
                std::process::exit(1);
            }
        }
        "dtrace" => {
            // steady full throttle on the dyno, sampled every 2 ms: plenum boost, then per turbo
            // compressor outlet boost, shaft speed, wastegate opening and controller duty
            let s = engine_arg(pos[1], bar, &extra);
            let rpm: f64 = flag("--rpm").map_or(4000.0, |x| x.parse().unwrap());
            let secs: f64 = flag("--secs").map_or(8.0, |x| x.parse().unwrap());
            let mut sys = System::build(&s, &q, 12345);
            sys.mode = Mode::Dyno;
            sys.omega = rpm * PI / 30.0;
            sys.throttle_cmd = 1.0;
            let every = (0.002 * q.rate) as usize;
            for k in 0..(secs * q.rate) as usize {
                sys.advance();
                if k % every == 0 {
                    let mut line = format!("{:.3},{:.2}", k as f64 / q.rate, (sys.vols[sys.plenum.unwrap()].p() - P_ATM) / 1e3);
                    for t in &sys.turbos {
                        let (x, d) = t.wg.as_ref().map_or((0.0, 0.0), |w| (w.x, w.ctrl.as_ref().map_or(0.0, |c| c.duty)));
                        line += &format!(",{:.2},{:.0},{:.3},{:.3}", (sys.vols[t.d].p() - P_ATM) / 1e3, t.shaft_rpm(), x, d);
                    }
                    println!("{line}");
                }
            }
        }
        "trace" => {
            let s = engine_arg(pos[1], bar, &extra);
            let (line, _, tr) = rev_run(&s, &q, 7, 3.0);
            println!("t,rpm,boost_kpa,shaft_rpm,surge_margin,bov");
            for r in tr {
                println!("{:.3},{:.0},{:.2},{:.0},{:.3},{:.3}", r[0], r[1], r[2], r[3], r[4], r[5]);
            }
            eprintln!("{line}");
        }
        "all" => {
            // every library engine that can take a turbo, with the generic test turbo, at the
            // defaults and with every fueling setting at its harshest
            let b = bar.unwrap_or(0.6);
            let (mut n_ok, mut n) = (0, 0);
            for id in spec::names() {
                let base = load_spec(id);
                if base.itb || base.is_two_stroke() || !base.chamber.is_empty() {
                    continue;
                }
                let s = if base.boost.as_ref().map_or(false, |x| x.turbo.is_some()) { base.clone() } else if base.boost.is_some() { continue } else { generic(&base, b, &extra) };
                for harsh in [false, true] {
                    let (line, ok, _) = rev_run_with(&s, &q, 7, 3.0, harsh);
                    n += 1;
                    n_ok += ok as usize;
                    println!("{} {:<28} {:<8} {}", if ok { "PASS" } else { "FAIL" }, id, if harsh { "harsh" } else { "default" }, line);
                }
            }
            println!("{n_ok} of {n} runs pass (plenum engines with the generic test turbo at {b} bar; 'harsh' = phi 1.7, overrun fueling 1, spark-cut limiter, lift-off pops 1)");
        }
        "lift" => {
            let s = engine_arg(pos[1], bar, &extra);
            let rpm: f64 = flag("--rpm").map_or(0.6 * s.redline, |x| x.parse().unwrap());
            let mut all = true;
            for b in ["vent", "recirc", "none"] {
                let (line, ok) = lift(&s, &q, rpm.round(), b);
                println!("{} {}", if ok { "PASS" } else { "FAIL" }, line);
                all &= ok;
            }
            if !all {
                std::process::exit(1);
            }
        }
        "control" => {
            let mut s = engine_arg(pos[1], bar, &extra);
            s.boost.as_mut().unwrap().turbo.as_mut().unwrap().boost_control = true;
            let target = s.boost.as_ref().unwrap().boost_bar * 100.0;
            let rpms: Vec<f64> = match flag("--rpms") {
                Some(l) => l.split(',').map(|x| x.parse().unwrap()).collect(),
                None => (3..=8).map(|i| (s.redline * i as f64 / 8.0 / 100.0).round() * 100.0).collect(),
            };
            let mut all = true;
            for &rpm in &rpms {
                let p = tpoint(&s, &q, rpm, 6.0, 1.0);
                let reach = p.wg > 0.02;
                let ok = !reach || (p.boost_kpa / target - 1.0).abs() < 0.05;
                all &= ok;
                println!("{} {:.0} rpm: boost {:.1} kPa against the {:.0} kPa target ({:+.1}%), wastegate {:.2} open, swing {:.2} kPa{}", if ok { "PASS" } else { "FAIL" }, rpm, p.boost_kpa, target, 100.0 * (p.boost_kpa / target - 1.0), p.wg, p.boost_swing, if reach { "" } else { "  (turbo cannot reach the target here; gate shut)" });
            }
            if !all {
                std::process::exit(1);
            }
        }
        "whistle" => {
            let s = engine_arg(pos[1], bar, &extra);
            let gain: f64 = flag("--gain").map_or(1.0, |x| x.parse().unwrap());
            let rpms: Vec<f64> = match flag("--rpms") {
                Some(l) => l.split(',').map(|x| x.parse().unwrap()).collect(),
                None => vec![0.4 * s.redline, 0.8 * s.redline],
            };
            let ok = whistle_check(&s, &q, &rpms, gain);
            println!("{} whistle partials at blade-pass frequencies of the simulated shaft speed, none above 0.45 of the inner rate or 20 kHz", if ok { "PASS" } else { "FAIL" });
        }
        _ => eprintln!("usage: engsim turbo rig [engine] | curve <engine> [--rpms a,b] [--warm s] | spool <engine> [--rpm r] | stability <engine> [--qualities a,b] | trace <engine> | lift <engine> [--rpm r] | control <engine> | whistle <engine> [--gain g]   (--generic <bar> puts the generic test turbo on an engine; --set '{{turbo fields}}' adjusts it; --quality q)"),
    }
}
