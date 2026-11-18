//! Checks for lift-off pops: `engsim liftoff ...` (see the usage text in `run`).
//! Everything here is a numeric check; nobody has listened.

use super::*;

/// One ignition in an exhaust collector.
pub struct Pop {
    /// seconds after the lift
    pub t: f64,
    /// its heat, as a share of one cylinder's full-load charge burning completely
    pub q: f64,
    /// loudest 10 ms at the exhaust microphone in the 30 ms after it, dB against the loudest
    /// 10 ms of the same engine at full throttle
    pub db: f64,
}

#[derive(Clone)]
pub struct Setup {
    pub level: f64,
    pub phi: f64,
    pub overrun: f64,
    pub limiter: Limiter,
    /// held speed before the lift, as a share of the redline, and the throttle held there
    pub from: f64,
    pub throttle: f64,
    pub hold: f64,
    pub after: f64,
    pub seed: u64,
}

impl Default for Setup {
    fn default() -> Self {
        Setup { level: 1.0, phi: 1.0, overrun: 0.0, limiter: Limiter::FuelCut, from: 0.8, throttle: 1.0, hold: 2.0, after: 3.0, seed: 71 }
    }
}

pub struct LiftRun {
    pub pops: Vec<Pop>,
    /// peaks found in the sound alone: (seconds after the lift, dB against full throttle)
    pub heard: Vec<(f64, f64)>,
    pub finite: bool,
    /// loudest 10 ms in the last second of the hold, dB SPL
    pub hold_db: f64,
    /// rpm every 100 inner samples after the lift
    pub rpm: Vec<f64>,
    /// exhaust microphone power after the lift, per inner sample
    pub e: Vec<f64>,
    /// [exhaust L, exhaust R, intake, block] at 48 kHz from 0.5 s before the lift
    pub audio: [Vec<f32>; 4],
    pub log: Vec<[f64; 5]>,
    /// film-fed cycles after the lift, and how many of them lit
    pub film_cycles: usize,
    pub film_fired: usize,
}

fn db_of(power: f64) -> f64 {
    10.0 * (power / 4e-10).max(1e-24).log10()
}

/// Mean power over windows of `w` samples every `hop` samples.
fn windows(e: &[f64], w: usize, hop: usize) -> Vec<f64> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + w <= e.len() {
        out.push(e[i..i + w].iter().sum::<f64>() / w as f64);
        i += hop;
    }
    out
}

/// Peaks in the sound: 5 ms windows every 1 ms that stand `over` dB above the median level of
/// the 300 ms around them, the loudest within 20 ms of each other. Returns (time, dB SPL).
pub fn heard_peaks(e: &[f64], rate: f64, over: f64) -> Vec<(f64, f64)> {
    let hop = (0.001 * rate) as usize;
    let env: Vec<f64> = windows(e, 5 * hop, hop).into_iter().map(db_of).collect();
    let mut out: Vec<(f64, f64)> = Vec::new();
    for i in 0..env.len() {
        let (a, b) = (i.saturating_sub(150), (i + 150).min(env.len()));
        let mut w: Vec<f64> = env[a..b].to_vec();
        w.sort_by(|x, y| x.partial_cmp(y).unwrap());
        let med = w[w.len() / 2];
        let (la, lb) = (i.saturating_sub(20), (i + 21).min(env.len()));
        if env[i] > med + over && env[la..lb].iter().all(|&x| x <= env[i]) {
            let t = (i as f64 + 2.5) * 0.001;
            if out.last().map_or(true, |l| t - l.0 > 0.020) {
                out.push((t, env[i]));
            }
        }
    }
    out
}

/// Hold the engine on the dyno, then let it go with the throttle shut.
pub fn lift_run(s: &spec::EngineSpec, q: &Quality, su: &Setup, ref_db: Option<f64>, keep_audio: bool, trace: bool) -> LiftRun {
    let mut sys = System::build(s, q, su.seed);
    sys.liftoff = su.level;
    sys.phi = su.phi;
    sys.overrun_fuel = su.overrun;
    sys.limiter_kind = su.limiter;
    sys.mode = Mode::Dyno;
    sys.omega = su.from * s.redline * PI / 30.0;
    sys.throttle_cmd = su.throttle;
    let k = (q.rate / 48_000.0).round() as usize;
    let mut dec = [audio::Decim::new(k), audio::Decim::new(k), audio::Decim::new(k), audio::Decim::new(k)];
    let mut au: [Vec<f32>; 4] = Default::default();
    let n_hold = (su.hold * q.rate) as usize;
    let mut hold_e = Vec::new();
    let mut finite = true;
    for i in 0..n_hold {
        let o = sys.advance();
        finite &= o.iter().all(|v| v.is_finite());
        if i + (q.rate as usize) >= n_hold {
            hold_e.push(0.5 * (o[0] * o[0] + o[1] * o[1]));
        }
        if keep_audio && i + (q.rate as usize) / 2 >= n_hold {
            for c in 0..4 {
                if let Some(v) = dec[c].push(o[c]) {
                    au[c].push(v as f32);
                }
            }
        }
    }
    let hold_db = peak_level(&hold_e, q.rate);
    let ref_db = ref_db.unwrap_or(hold_db);
    let charge_q = s.heat_q * RHO_ATM * s.disp_cyl();
    sys.mode = Mode::Free;
    sys.throttle_cmd = 0.0;
    sys.lift_log = Some(Vec::new());
    let _ = trace;
    let t0 = sys.t;
    let n = (su.after * q.rate) as usize;
    let mut e = Vec::with_capacity(n);
    let mut rpm = Vec::with_capacity(n / 100 + 1);
    let mut raw: Vec<(f64, f64)> = Vec::new();
    let mut qs: Vec<f64> = sys.af.iter().map(|a| a.q_sum).collect();
    for i in 0..n {
        let o = sys.advance();
        finite &= o.iter().all(|v| v.is_finite()) && sys.omega.is_finite();
        e.push(0.5 * (o[0] * o[0] + o[1] * o[1]));
        for (g, a) in sys.af.iter().enumerate() {
            if a.q_sum > qs[g] {
                raw.push((i as f64 / q.rate, (a.q_sum - qs[g]) / charge_q));
                qs[g] = a.q_sum;
            }
        }
        if i % 100 == 0 {
            rpm.push(sys.rpm());
        }
        if keep_audio {
            for c in 0..4 {
                if let Some(v) = dec[c].push(o[c]) {
                    au[c].push(v as f32);
                }
            }
        }
    }
    let w = (0.010 * q.rate) as usize;
    let hop = (0.001 * q.rate) as usize;
    let pops = raw
        .iter()
        .map(|&(t, qq)| {
            let i0 = (t * q.rate) as usize;
            let i1 = (i0 + (0.030 * q.rate) as usize + w).min(e.len());
            let best = windows(&e[i0.min(e.len())..i1], w, hop).into_iter().fold(0.0, f64::max);
            Pop { t, q: qq, db: db_of(best) - ref_db }
        })
        .collect();
    let heard = heard_peaks(&e, q.rate, 8.0).into_iter().map(|(t, d)| (t, d - ref_db)).collect();
    let log: Vec<[f64; 5]> = sys.lift_log.take().unwrap_or_default().into_iter().map(|mut r| { r[0] -= t0; r }).collect();
    let (film_cycles, film_fired) = (log.len(), log.iter().filter(|r| r[4] > 0.0).count());
    LiftRun { pops, heard, finite, hold_db, rpm, e, audio: au, log, film_cycles, film_fired }
}

/// Full-throttle reference: loudest 10 ms on the dyno at 80% of the redline.
pub fn full_throttle_db(s: &spec::EngineSpec, q: &Quality) -> f64 {
    lift_run(s, q, &Setup { level: 0.0, after: 0.01, ..Default::default() }, None, false, false).hold_db
}

fn fmt_pops(p: &[Pop]) -> String {
    if p.is_empty() {
        return "none".into();
    }
    p.iter().map(|x| format!("{:.0} ms {:+.1} dB ({:.2})", 1000.0 * x.t, x.db, x.q)).collect::<Vec<_>>().join(", ")
}

fn gaps(p: &[Pop]) -> String {
    if p.len() < 2 {
        return "-".into();
    }
    p.windows(2).map(|w| format!("{:.0}", 1000.0 * (w[1].t - w[0].t))).collect::<Vec<_>>().join(" ")
}

fn describe(name: &str, r: &LiftRun) {
    // ignitions within 30 ms of the first of a group are heard as one bang
    let mut bangs: Vec<(f64, usize, f64)> = Vec::new();
    for p in &r.pops {
        match bangs.last_mut() {
            Some(b) if p.t - b.0 < 0.030 => {
                b.1 += 1;
                b.2 = b.2.max(p.db);
            }
            _ => bangs.push((p.t, 1, p.db)),
        }
    }
    let loud = r.pops.iter().map(|p| p.db).fold(f64::MIN, f64::max);
    let loud_t = r.pops.iter().find(|p| p.db == loud).map_or(0.0, |p| p.t);
    if r.pops.is_empty() {
        println!("  {name}: no ignitions ({} film-fed cycles, {} lit); peaks in the sound alone: {}", r.film_cycles, r.film_fired, r.heard.len());
        return;
    }
    println!("  {name}: {} ignitions, {} bangs; last at {:.0} ms; loudest {loud:+.1} dB at {:.0} ms", r.pops.len(), bangs.len(), 1000.0 * r.pops.last().unwrap().t, 1000.0 * loud_t);
    if r.pops.len() <= 16 {
        println!("      each (ms after the lift, dB against full throttle, heat as a share of one full cylinder charge): {}", fmt_pops(&r.pops));
        println!("      gaps, ms: {}", gaps(&r.pops));
    } else {
        let g: Vec<f64> = r.pops.windows(2).map(|w| 1000.0 * (w[1].t - w[0].t)).collect();
        let mut sg = g.clone();
        sg.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let per: Vec<String> = (0..6).map(|k| r.pops.iter().filter(|p| p.t >= 0.25 * k as f64 && p.t < 0.25 * (k + 1) as f64).count().to_string()).collect();
        let heat = r.pops.iter().map(|p| p.q).sum::<f64>() / r.pops.len() as f64;
        println!("      gaps, ms: median {:.0}, longest {:.0}; ignitions in each quarter second: {}; mean heat {:.2} of a cylinder charge", sg[sg.len() / 2], sg[sg.len() - 1], per.join(" "), heat);
    }
    println!("      film-fed cycles {}, {} lit; peaks in the sound alone (5 ms windows 8 dB over the median of the 300 ms around them): {}", r.film_cycles, r.film_fired, if r.heard.is_empty() { "none".into() } else { r.heard.iter().map(|h| format!("{:.0} ms {:+.1} dB", 1000.0 * h.0, h.1)).collect::<Vec<_>>().join(", ") });
}

fn write_outputs(r: &LiftRun, wav: Option<&String>, csv: Option<&String>, rate: f64) {
    if let Some(path) = wav {
        // the live app's mix (exhaust, plus intake and block in the centre), at one fixed gain
        // that puts the held full throttle's peak at 0.25, with a soft knee above
        let n = r.audio[0].len();
        let mut mix = Vec::with_capacity(2 * n);
        for i in 0..n {
            let c = 0.707 * (0.6 * r.audio[2][i] + 2.0 * r.audio[3][i]);
            mix.push(r.audio[0][i] + c);
            mix.push(r.audio[1][i] + c);
        }
        let pk = mix[..(2 * 24_000).min(mix.len())].iter().fold(0.0f32, |a, x| a.max(x.abs())).max(1e-9);
        for x in mix.iter_mut() {
            *x = (*x * 0.25 / pk).tanh();
        }
        audio::write_wav16(Path::new(path), 2, 48_000, &mix).unwrap();
    }
    if let Some(path) = csv {
        let mut out = String::from("kind,t,value\n");
        let hop = (0.001 * rate) as usize;
        for (i, v) in windows(&r.e, 5 * hop, hop).into_iter().enumerate() {
            out.push_str(&format!("level,{:.4},{:.2}\n", (i as f64 + 2.5) * 0.001, db_of(v) - r.hold_db));
        }
        for p in &r.pops {
            out.push_str(&format!("pop,{:.4},{:.2}\n", p.t, p.db));
        }
        for (i, v) in r.rpm.iter().enumerate() {
            if i % 10 == 0 {
                out.push_str(&format!("rpm,{:.4},{:.0}\n", i as f64 * 100.0 / rate, v));
            }
        }
        std::fs::write(path, out).unwrap();
    }
}

fn parallel<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let results = std::sync::Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(10);
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| loop {
                let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if k >= n {
                    break;
                }
                let r = f(k);
                results.lock().unwrap().push((k, r));
            });
        }
    });
    let mut v = results.into_inner().unwrap();
    v.sort_by_key(|r| r.0);
    v.into_iter().map(|r| r.1).collect()
}

/// Loudest pop allowed against the engine's own full throttle, dB (see docs/physics.md, "Lift-off pops").
pub const POP_BOUND_DB: f64 = 20.0;

fn all_engines(q: &Quality, qname: &str, level: f64) {
    let names: Vec<&str> = spec::names().to_vec();
    println!("lift-off at level {level} for all {} engines, quality {qname}: full throttle on the dyno at 80% of the redline for 2 s, throttle shut, 8 s free; the same at level 0 beside it", names.len());
    println!("idle: mean speed over the last 2 s against the idle target; a pop is an ignition in a collector; dB against the engine's loudest 10 ms at full throttle");
    let res = parallel(names.len() * 2, |k| {
        let s = load_spec(names[k / 2]);
        let su = Setup { level: if k % 2 == 0 { 0.0 } else { level }, after: 8.0, ..Default::default() };
        let r = lift_run(&s, q, &su, None, false, false);
        let per = r.rpm.len() as f64 / 8.0;
        let tail = &r.rpm[(6.0 * per) as usize..];
        let idle = tail.iter().sum::<f64>() / tail.len() as f64 / s.idle;
        let lo = tail.iter().cloned().fold(f64::MAX, f64::min) / s.idle;
        // seconds until the speed first comes within 20% of idle
        let t_idle = r.rpm.iter().position(|&x| x < 1.2 * s.idle).map_or(f64::NAN, |i| i as f64 / per);
        // level in the last 2 s against the level-0 run is taken by the caller; here the loudest 10 ms after 3 s
        let late_db = peak_level(&r.e[(3.0 * q.rate) as usize..], q.rate) - r.hold_db;
        (r, idle, lo, t_idle, late_db, s.kind.clone())
    });
    let (mut bad, mut zero, mut many, mut loud) = (Vec::new(), Vec::new(), Vec::new(), Vec::<String>::new());
    let (mut abs, mut rel) = (Vec::new(), Vec::new());
    let mut hist = std::collections::BTreeMap::<usize, usize>::new();
    for (i, pair) in res.chunks(2).enumerate() {
        let (r0, r1) = (&pair[0], &pair[1]);
        let r = &r1.0;
        let n = r.pops.len();
        *hist.entry(n).or_default() += 1;
        let mx = r.pops.iter().map(|p| p.db).fold(f64::MIN, f64::max);
        let last = r.pops.last().map_or(0.0, |p| p.t);
        let ok = r.finite && r0.0.finite && (r1.1 - r0.1).abs() < 0.08 && r1.2 > (r0.2 - 0.1).min(0.6);
        println!(
            "  {} {:<26} {:<8} pops {:>2}{} last {:>5.0} ms, loudest {:>6}; idle/target {:.2} (level 0: {:.2}), lowest {:.2} (level 0: {:.2}); reaches idle after {:.1} s (level 0: {:.1}); loudest 10 ms after 3 s {:+.1} dB (level 0: {:+.1}); level-0 pops {}",
            if ok { "ok " } else { "BAD" },
            names[i],
            r1.5,
            n,
            if n > 0 { format!(" [{}]", r.pops.iter().map(|p| format!("{:.0}", 1000.0 * p.t)).collect::<Vec<_>>().join(" ")) } else { String::new() },
            1000.0 * last,
            if n > 0 { format!("{mx:+.1}") } else { "-".into() },
            r1.1,
            r0.1,
            r1.2,
            r0.2,
            r1.3,
            r0.3,
            r1.4,
            r0.4,
            r0.0.pops.len()
        );
        if !ok {
            bad.push(names[i]);
        }
        if n == 0 {
            zero.push(names[i]);
        }
        if n > 12 {
            many.push(names[i]);
        }
        if n > 0 && mx > POP_BOUND_DB {
            loud.push(format!("{} ({:+.1} dB: full throttle {:.0} dB SPL, pop {:.0} dB SPL)", names[i], mx, r.hold_db, r.hold_db + mx));
        }
        if n > 0 {
            abs.push(r.hold_db + mx);
            rel.push(mx);
        }
    }
    println!("  pops per engine (count: engines): {}", hist.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(", "));
    println!("  zero pops ({}): {}", zero.len(), zero.join(" "));
    println!("  more than twelve ({}): {}", many.len(), many.join(" "));
    println!("  louder than +{POP_BOUND_DB} dB over full throttle ({}): {}", loud.len(), loud.join("; "));
    let med = |v: &mut Vec<f64>| { v.sort_by(|a, b| a.partial_cmp(b).unwrap()); (v[0], v[v.len() / 2], v[v.len() - 1]) };
    if !rel.is_empty() {
        let (r, a) = (med(&mut rel), med(&mut abs));
        println!("  loudest pop of each engine that popped: against its full throttle {:+.1} / {:+.1} / {:+.1} dB (lowest / median / highest); in absolute terms {:.0} / {:.0} / {:.0} dB SPL at 1 m", r.0, r.1, r.2, a.0, a.1, a.2);
    }
    println!("  {} all finite, idle within 8% of the level-0 run's and never under 0.6 of target in the last 2 s (or, where the level-0 run dips lower, within 0.1 of its lowest){}", if bad.is_empty() { "PASS" } else { "FAIL" }, if bad.is_empty() { String::new() } else { format!(": {}", bad.join(" ")) });
    if !bad.is_empty() {
        std::process::exit(1);
    }
}

fn drive(s: &spec::EngineSpec, q: &Quality, level: f64) {
    println!("drive, {}: the live app's generic car from rest, full throttle through first, second and third with automatic shifts, lift in fourth at 80% of the redline, coast 3 s; level {level}", s.name);
    let mut sys = System::build(s, q, 81);
    sys.liftoff = level;
    let mut car = generic_car(s, 0, 0.0);
    car.auto = true;
    car.locked = false;
    car.v = 0.0;
    sys.mode = Mode::Drive(car);
    let charge_q = s.heat_q * RHO_ATM * s.disp_cyl();
    let mut qs: Vec<f64> = sys.af.iter().map(|a| a.q_sum).collect();
    let (mut gear, mut lifted) = (0usize, f64::NAN);
    let mut shifts: Vec<(f64, usize, f64)> = Vec::new();
    let mut pops: Vec<(f64, f64)> = Vec::new();
    let mut e = Vec::new();
    let mut finite = true;
    let mut i = 0usize;
    loop {
        let t = i as f64 / q.rate;
        let g = if let Mode::Drive(d) = &sys.mode { d.gear } else { 0 };
        if g != gear {
            shifts.push((t, g + 1, sys.rpm()));
            gear = g;
        }
        if lifted.is_nan() && g == 3 && sys.rpm() > 0.8 * s.redline && !matches!(&sys.mode, Mode::Drive(d) if d.shifting > 0.0) {
            lifted = t;
        }
        sys.throttle_cmd = if t < 0.5 || !lifted.is_nan() { 0.0 } else { 1.0 };
        let o = sys.advance();
        finite &= o.iter().all(|v| v.is_finite());
        e.push(0.5 * (o[0] * o[0] + o[1] * o[1]));
        for (gi, a) in sys.af.iter().enumerate() {
            if a.q_sum > qs[gi] {
                pops.push((t, (a.q_sum - qs[gi]) / charge_q));
                qs[gi] = a.q_sum;
            }
        }
        i += 1;
        if (!lifted.is_nan() && t > lifted + 3.0) || t > 90.0 {
            break;
        }
    }
    let ft = full_throttle_db(s, q);
    let w = (0.010 * q.rate) as usize;
    let hop = (0.001 * q.rate) as usize;
    let lvl = |t: f64| {
        let i0 = (t * q.rate) as usize;
        let i1 = (i0 + (0.030 * q.rate) as usize + w).min(e.len());
        db_of(windows(&e[i0..i1], w, hop).into_iter().fold(0.0, f64::max)) - ft
    };
    let list = |a: f64, b: f64| {
        let v: Vec<String> = pops.iter().filter(|p| p.0 >= a && p.0 < b).map(|p| format!("{:.0} ms {:+.1} dB ({:.2})", 1000.0 * (p.0 - a), lvl(p.0), p.1)).collect();
        if v.is_empty() { "none".to_string() } else { format!("{}: {}", v.len(), v.join(", ")) }
    };
    for (k, sh) in shifts.iter().enumerate() {
        let end = shifts.get(k + 1).map_or(lifted, |n| n.0).min(sh.0 + 1.0);
        println!("  shift into gear {} at {:.2} s ({:.0} rpm): pops in the next {:.2} s: {}", sh.1, sh.0, sh.2, end - sh.0, list(sh.0, end));
    }
    let between: usize = pops.iter().filter(|p| p.0 < lifted && !shifts.iter().any(|s| p.0 >= s.0 && p.0 < s.0 + 1.0)).count();
    println!("  pops under power away from a shift: {between}{}", if between > 0 { format!(" (at s: {})", pops.iter().filter(|p| p.0 < lifted && !shifts.iter().any(|s| p.0 >= s.0 && p.0 < s.0 + 1.0)).take(12).map(|p| format!("{:.2}", p.0)).collect::<Vec<_>>().join(" ")) } else { String::new() });
    println!("  final lift at {:.2} s: {}", lifted, list(lifted, lifted + 3.0));
    println!("  finite: {finite}");
}

fn stress(q: &Quality) {
    println!("stress: lift-off pops 1 with phi 1.7, overrun fueling 1 and the spark-cut limiter, free in neutral for 20 s: full throttle 1.2 s (into the limiter), shut 0.8 s, repeated; then 4 s shut");
    let mut fails = 0;
    for id in ["v8_cross", "i4", "bmw_s70_2_v12"] {
        let s = load_spec(id);
        let mut sys = System::build(&s, q, 91);
        sys.liftoff = 1.0;
        sys.phi = 1.7;
        sys.overrun_fuel = 1.0;
        sys.limiter_kind = Limiter::SparkCut;
        let r = af_run(&mut sys, 20.0, |sy, t| sy.throttle_cmd = if t < 16.0 && t % 2.0 < 1.2 { 1.0 } else { 0.0 });
        let per = r.rpm.len() as f64 / 20.0;
        let tail = &r.rpm[(18.5 * per) as usize..];
        let lo = tail.iter().cloned().fold(f64::MAX, f64::min);
        let hi = r.rpm.iter().cloned().fold(0.0, f64::max);
        let pk = peak_level(&r.e, q.rate) - full_throttle_db(&s, q);
        let film = sys.cyls.iter().map(|c| c.film).fold(0.0, f64::max) / (RHO_ATM * s.disp_cyl());
        let ok = r.finite && lo > 0.5 * s.idle && film.is_finite();
        println!("  {} {:<14} finite {}; {} ignitions; highest speed {:.0} (redline {:.0}); lowest in the last 1.5 s {:.0} (idle {:.0}); loudest 10 ms {:+.1} dB over full throttle; film left {:.3} of a charge", if ok { "PASS" } else { "FAIL" }, id, r.finite, r.events.len(), hi, s.redline, lo, s.idle, pk, film);
        if !ok {
            fails += 1;
        }
    }
    if fails > 0 {
        std::process::exit(1);
    }
}

pub fn run(args: &[String]) {
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let num = |name: &str, d: f64| flag(name).and_then(|x| x.parse().ok()).unwrap_or(d);
    let has = |name: &str| args.iter().any(|a| a == name);
    let mut pos: Vec<&String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with("--") {
            i += if args[i] == "--trace" { 1 } else { 2 };
        } else {
            pos.push(&args[i]);
            i += 1;
        }
    }
    if pos.is_empty() {
        eprintln!("usage: engsim liftoff <engine|all|table|drive <engine>|stress> [quality] [level] [--phi x] [--overrun x] [--limiter spark] [--from share-of-redline] [--throttle x] [--after s] [--wav file] [--csv file] [--trace]");
        return;
    }
    let (what, rest) = if pos[0] == "drive" { ("drive", &pos[1..]) } else { (pos[0].as_str(), &pos[..]) };
    let qname = rest.get(1).map_or("live96+W", |s| s.as_str());
    let q = quality(qname);
    let level: f64 = rest.get(2).and_then(|x| x.parse().ok()).unwrap_or(1.0);
    match what {
        "all" => all_engines(&q, qname, level),
        "stress" => stress(&q),
        "drive" => drive(&load_spec(rest[0]), &q, level),
        "stats" => {
            // the spread over random streams: `liftoff stats <quality> <level> [--n 12] [engines via --engines a,b]`
            let n = num("--n", 12.0) as usize;
            let ids: Vec<String> = flag("--engines").unwrap_or_else(|| "v8_cross,i4".into()).split(',').map(|x| x.to_string()).collect();
            println!("spread over {n} random streams, quality {qname}: ignitions after a lift from full throttle at 80% of the redline (count mean [min..max]; share of runs with none; mean time of the first and last, ms; mean heat of the 1st, 2nd, ... as shares of a cylinder charge; share of runs whose loudest is in the first 0.3 s; mean loudest dB over full throttle)");
            for id in &ids {
                let s = load_spec(id);
                let ft = full_throttle_db(&s, &q);
                for lv in [0.3, 0.6, 1.0] {
                    let rs = parallel(n, |k| lift_run(&s, &q, &Setup { level: lv, seed: 1000 + 17 * k as u64, ..Default::default() }, Some(ft), false, false));
                    let c: Vec<usize> = rs.iter().map(|r| r.pops.len()).collect();
                    let mean = c.iter().sum::<usize>() as f64 / n as f64;
                    let with: Vec<&LiftRun> = rs.iter().filter(|r| !r.pops.is_empty()).collect();
                    let m = |f: &dyn Fn(&LiftRun) -> f64| if with.is_empty() { f64::NAN } else { with.iter().map(|r| f(r)).sum::<f64>() / with.len() as f64 };
                    let order: Vec<String> = (0..8).map(|k| { let v: Vec<f64> = rs.iter().filter_map(|r| r.pops.get(k).map(|p| p.q)).collect(); if v.is_empty() { "-".into() } else { format!("{:.2}", v.iter().sum::<f64>() / v.len() as f64) } }).collect();
                    let early = m(&|r| { let mx = r.pops.iter().map(|p| p.q).fold(0.0, f64::max); if r.pops.iter().find(|p| p.q == mx).unwrap().t < 0.3 { 1.0 } else { 0.0 } });
                    println!("  {:<10} level {:.1}: count {:.1} [{}..{}], none in {:.0}%; first {:.0} last {:.0}; heat by order {}; loudest early {:.0}%; loudest {:+.1} dB (max {:+.1}); counts {:?}", id, lv, mean, c.iter().min().unwrap(), c.iter().max().unwrap(), 100.0 * (n - with.len()) as f64 / n as f64, 1000.0 * m(&|r| r.pops[0].t), 1000.0 * m(&|r| r.pops.last().unwrap().t), order.join(" "), 100.0 * early, m(&|r| r.pops.iter().map(|p| p.db).fold(f64::MIN, f64::max)), rs.iter().flat_map(|r| r.pops.iter().map(|p| p.db)).fold(f64::NAN, f64::max), c);
                }
            }
        }
        "table" => {
            // the standard table: both reference engines at 0, 0.3 and 1, the low-load lift, and
            // overrun fueling on the same test
            println!("lift-off table, quality {qname}: full throttle on the dyno at 80% of the redline for 2 s, throttle shut, 3 s free in neutral");
            let ids = ["v8_cross", "i4"];
            let jobs: Vec<(usize, &str, Setup)> = ids
                .iter()
                .flat_map(|&id| {
                    [
                        ("lift-off pops 0", Setup { level: 0.0, ..Default::default() }),
                        ("lift-off pops 0.3", Setup { level: 0.3, ..Default::default() }),
                        ("lift-off pops 0.6", Setup { level: 0.6, ..Default::default() }),
                        ("lift-off pops 1", Setup::default()),
                        ("lift-off pops 1 from 2,000 rpm at 15% throttle", Setup { from: -2000.0, throttle: 0.15, ..Default::default() }),
                        ("lift-off pops 1 from 50% of the redline at full throttle", Setup { from: 0.5, ..Default::default() }),
                        ("lift-off pops 1 from 80% of the redline at 30% throttle", Setup { throttle: 0.3, ..Default::default() }),
                        ("overrun fueling 0.3, lift-off pops 0", Setup { level: 0.0, overrun: 0.3, ..Default::default() }),
                        ("overrun fueling 1, lift-off pops 0", Setup { level: 0.0, overrun: 1.0, ..Default::default() }),
                        ("overrun fueling 0.3 with lift-off pops 1", Setup { overrun: 0.3, ..Default::default() }),
                        ("phi 1.3 with lift-off pops 1", Setup { phi: 1.3, ..Default::default() }),
                    ]
                    .into_iter()
                    .map(move |(n, s)| (ids.iter().position(|x| *x == id).unwrap(), n, s))
                })
                .collect();
            let refs: Vec<f64> = ids.iter().map(|id| full_throttle_db(&load_spec(id), &q)).collect();
            let res = parallel(jobs.len(), |k| {
                let s = load_spec(ids[jobs[k].0]);
                let mut su = jobs[k].2.clone();
                if su.from < 0.0 {
                    su.from = -su.from / s.redline;
                }
                lift_run(&s, &q, &su, Some(refs[jobs[k].0]), false, false)
            });
            let mut last = usize::MAX;
            for (j, r) in jobs.iter().zip(&res) {
                if j.0 != last {
                    println!("{} (full throttle at 80% of the redline: loudest 10 ms {:.1} dB SPL at 1 m)", ids[j.0], refs[j.0]);
                    last = j.0;
                }
                describe(j.1, r);
            }
        }
        id => {
            let s = load_spec(id);
            let mut from = num("--from", 0.8);
            if from > 1.5 {
                from /= s.redline;
            }
            let su = Setup { level, phi: num("--phi", 1.0), overrun: num("--overrun", 0.0), limiter: if flag("--limiter").as_deref() == Some("spark") { Limiter::SparkCut } else { Limiter::FuelCut }, from, throttle: num("--throttle", 1.0), hold: 2.0, after: num("--after", 3.0), seed: num("--seed", 71.0) as u64 };
            let (wav, csv) = (flag("--wav"), flag("--csv"));
            let ft = full_throttle_db(&s, &q);
            let r = lift_run(&s, &q, &su, Some(ft), wav.is_some(), has("--trace"));
            println!("{id}, quality {qname}: {:.0} rpm at throttle {} for 2 s on the dyno, then shut and free for {} s; lift-off pops {}, phi {}, overrun fueling {}; full throttle reference {:.1} dB SPL; finite {}", su.from * s.redline, su.throttle, su.after, su.level, su.phi, su.overrun, ft, r.finite);
            describe("result", &r);
            let per = r.rpm.len() as f64 / su.after;
            println!("      rpm at 0.2 s steps: {}", (0..(su.after * 10.0) as usize).step_by(2).map(|k| format!("{:.0}", r.rpm[((k as f64 / 10.0) * per) as usize])).collect::<Vec<_>>().join(" "));
            if has("--trace") {
                println!("  film-fed cycles (first 60 and every 20th after): t ms, cycle phi, air / full charge, residual share, 1 if it lit");
                for (k, row) in r.log.iter().enumerate() {
                    if k < 60 || k % 20 == 0 {
                        println!("    {:>6.1} {:>6.2} {:>6.3} {:>5.2} {:>5.2}", 1000.0 * row[0], row[1], row[2], row[3], row[4]);
                    }
                }
            }
            write_outputs(&r, wav.as_ref(), csv.as_ref(), q.rate);
        }
    }
}
