//! A pipe solved along its characteristics instead of on a grid.
//!
//! In a uniform pipe of gas at one entropy, the two Riemann variables
//!     lambda = A + k U   (travels right at u + a)
//!     beta   = A - k U   (travels left  at a - u)
//! with A = a / a_ref = (p / p_atm)^((gamma-1)/(2 gamma)), U = u / a_ref, k = (gamma-1)/2,
//! are carried unchanged along their own characteristics. So a pipe needs no cells: each
//! direction is a delay line, and each sample's delay follows from its own speed. Faster
//! (higher-pressure) samples catch slower ones, which is how the wave steepens; where they
//! would overtake, arrival times are merged, which makes a shock one sample thick.
//! What this gives up against the grid solver: entropy is one value per pipe (no hot slugs),
//! friction and wall heat are lumped, and area changes are sudden steps at constant pressure.

use crate::gas::*;
use crate::pipe::{Bc, EndFlux, Rad, Sec};
use crate::prof::{self, cat, cnt};

const K: f64 = 0.5 * GM1;
const CAP: usize = 8192;
/// Exit restriction of a muffler chamber, as a loss coefficient on the tailpipe's dynamic head.
/// Chosen so the back-pressure matches what the grid solver's muffler produces.
pub const MUFFLER_ZETA: f64 = 7.0;
/// Corner frequency of a muffler's absorptive packing (overridable for tuning).
pub const MUFFLER_ABSORB_HZ: f64 = 600.0;
/// Plain pipe walls: boundary-layer (viscous and thermal) losses take a little of every sound
/// wave. Modelled as a shelf: above WALL_LOSS_HZ a crossing loses 1 - exp(-WALL_LOSS * len / radius).
/// 0.002 gives about 3.5% for 0.6 m of 70 mm pipe, the textbook order of magnitude near 2 kHz.
/// Without it a characteristic pipe has no loss at all at zero flow, and resonances between
/// two such pipes can build without limit.
pub const WALL_LOSS_HZ: f64 = 300.0;
pub const WALL_LOSS: f64 = 0.002;
fn wall_loss() -> f64 {
    std::env::var("ENGSIM_WALL_LOSS").ok().and_then(|v| v.parse().ok()).unwrap_or(WALL_LOSS)
}
/// FITTED, NOT PHYSICAL: treble loss along a pipe.
/// Pressure pulses steepen as they travel (the high-pressure part runs faster). At high load they
/// reach the tailpipe as shocks, which the open end radiates as clicks (the "crackle" of the live
/// levels). The physical terms here, band-limited reading (BAND_LIMIT) and the boundary-layer
/// loss (KIRCHHOFF), remove the aliasing at live48 but leave the shocks: at live96+W the 996 GT3
/// still has 655 one-sample steps in its exhaust and a 0.24 share above 8 kHz. The finite-volume
/// solver gets no shocks only because it damps waves shorter than about eight of its cells, which
/// is numerical. This term copies that: each crossing passes a second-order Butterworth
/// low-pass, loss rising as f^4 like a second-order grid's numerical dissipation, cutoff
/// TREBLE_HZ * (1 m / len)^(1/4) so the loss in dB grows with length. 5 kHz is fitted to the
/// live96 grid's spectra above 4 kHz. 0 turns it off.
pub const TREBLE_HZ: f64 = 5000.0;
fn treble_hz() -> f64 {
    std::env::var("ENGSIM_TREBLE_HZ").ok().and_then(|v| v.parse().ok()).unwrap_or(TREBLE_HZ)
}
/// How each delay line samples what arrives: 0 at one instant (a front between two reads lands
/// whole on the next one), 1 as the mean over one sample period, 2 as a triangle two periods
/// wide (the kernel of a polyBLEP). See `Line::read_avg`.
pub const BAND_LIMIT: u8 = 2;
fn band_limit() -> u8 {
    std::env::var("ENGSIM_BL").ok().and_then(|v| v.parse().ok()).unwrap_or(BAND_LIMIT)
}
/// Viscous and thermal boundary-layer loss of a duct (Kirchhoff 1868, the wide-tube limit; as quoted
/// in arXiv 1812.11063, reduced to this form by Claude):
/// a plane wave in a pipe of radius r loses alpha = sqrt(omega nu / 2) / (r c) * (1 + (gamma - 1)
/// / sqrt(Pr)) nepers per metre and is slowed by the same amount in phase, i.e. the propagation
/// constant gains (1 + i) alpha, which in the time domain is exp(-a sqrt(s)) per crossing with
/// a = len (1 + (gamma - 1) / sqrt(Pr)) sqrt(nu) / (r c). nu is the gas's kinematic viscosity
/// at the pipe's temperature (Sutherland's law for air, at atmospheric pressure). This replaces
/// WALL_LOSS's flat shelf in plain pipes (it was a rough stand-in for the same loss, matched near
/// 2 kHz). Classical bulk absorption (f^2) is under 2% of this at 10 kHz for these lengths and is
/// left out, as is molecular relaxation.
pub const KIRCHHOFF: bool = true;
fn kirchhoff_on() -> bool {
    std::env::var("ENGSIM_KIRCHHOFF").map(|v| v != "0").unwrap_or(KIRCHHOFF)
}
const PRANDTL: f64 = 0.71;

/// exp(-a sqrt(s)) for small a: 1 - a sqrt(s), with sqrt(s) built from first-order high-passes
/// s / (s + p_k) at poles 4^k x 20 Hz, weighted sqrt(p_k) ln(4) / pi (the integral
/// sqrt(s) = (1/pi) Int_0^inf p^(-1/2) s/(s+p) dp, sampled in ln p) and rescaled by 1/0.91.
/// Magnitude within 10% and phase within 10 degrees of sqrt(i omega) from 100 Hz to 20 kHz.
const KH_N: usize = 7;
#[derive(Clone, Copy, Default)]
struct Kirch {
    c: [f64; KH_N],
    w: [f64; KH_N],
    lp: [f64; KH_N],
}

impl Kirch {
    fn new(dt: f64, init: f64) -> Self {
        let mut k = Kirch { lp: [init; KH_N], ..Default::default() };
        for i in 0..KH_N {
            let p_ideal = 2.0 * std::f64::consts::PI * 20.0 * 4f64.powi(i as i32);
            k.w[i] = p_ideal.sqrt() * 4f64.ln() / std::f64::consts::PI / 0.91;
            let p = p_ideal.min(2.0 * std::f64::consts::PI * 0.45 / dt);
            k.c[i] = 1.0 - (-p * dt).exp();
        }
        k
    }
    #[inline]
    fn run(&mut self, x: f64, a: f64) -> f64 {
        let mut d = 0.0;
        for i in 0..KH_N {
            self.lp[i] += self.c[i] * (x - self.lp[i]);
            d += self.w[i] * (x - self.lp[i]);
        }
        x - a * d
    }
}

/// Kinematic viscosity of air at temperature t (K) and atmospheric pressure: Sutherland's law.
fn nu_air(t: f64) -> f64 {
    let mu = 1.458e-6 * t.powf(1.5) / (t + 110.4);
    mu * R_GAS * t / P_ATM
}

/// Fraction of the wave above the corner frequency absorbed per crossing of the chamber.
pub const MUFFLER_ABSORB: f64 = 0.2;
fn absorb_frac() -> f64 {
    std::env::var("ENGSIM_ABSORB").ok().and_then(|v| v.parse().ok()).unwrap_or(MUFFLER_ABSORB)
}
fn absorb_hz() -> f64 {
    std::env::var("ENGSIM_ABSORB_HZ").ok().and_then(|v| v.parse().ok()).unwrap_or(MUFFLER_ABSORB_HZ)
}

/// Second-order Butterworth low-pass (transposed direct form II), unity gain at zero frequency.
#[derive(Clone, Copy, Default)]
struct Lp2 {
    b0: f64,
    b1: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Lp2 {
    fn new(fc: f64, dt: f64, init: f64) -> Self {
        let k = (std::f64::consts::PI * fc * dt).tan();
        let n = 1.0 / (1.0 + std::f64::consts::SQRT_2 * k + k * k);
        let b0 = k * k * n;
        let a1 = 2.0 * (k * k - 1.0) * n;
        let a2 = (1.0 - std::f64::consts::SQRT_2 * k + k * k) * n;
        let mut f = Lp2 { b0, b1: 2.0 * b0, a1, a2, z1: 0.0, z2: 0.0 };
        // start settled at `init`
        f.z1 = init * (1.0 - b0);
        f.z2 = init * (b0 - a2);
        f
    }
    #[inline]
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b0 * x - self.a2 * y;
        y
    }
}

struct Line {
    val: Vec<f64>,
    arr: Vec<f64>,
    w: usize,
    r: usize,
    init: f64,
}

impl Line {
    fn new(init: f64) -> Self {
        Line { val: vec![init; CAP], arr: vec![0.0; CAP], w: 0, r: 0, init }
    }
    /// Add a sample emitted now (time `now`) that would arrive at `arr`.
    /// If it would overtake earlier samples, their arrival times are pooled to the mean
    /// (pool-adjacent-violators). Everything in a pool arrives at once, so the reader jumps
    /// across it: those samples are swallowed by the shock, which is how a shock dissipates.
    /// Samples due to arrive by `guard` are already being read and keep their arrival times.
    #[inline]
    fn push(&mut self, v: f64, arr: f64, now: f64, guard: f64) -> f64 {
        let m = CAP - 1;
        let mut mean = arr;
        let mut cnt = 1.0;
        let mut j = self.w; // pool covers [j, w]
        while j > self.r + 1 {
            let pv = self.arr[(j - 1) & m];
            if pv <= mean || pv <= guard {
                break;
            }
            let mut b = j - 1;
            while b > self.r + 1 && self.arr[(b - 1) & m] == pv {
                b -= 1;
            }
            let bc = (j - b) as f64;
            mean = (mean * cnt + pv * bc) / (cnt + bc);
            cnt += bc;
            j = b;
        }
        if j < self.w {
            let floor = self.arr[(j.max(1) - 1) & m].max(now);
            let mean = if j > 0 { mean.max(floor) } else { mean };
            for k in j..self.w {
                self.arr[k & m] = mean;
            }
            self.arr[self.w & m] = mean;
        } else {
            self.arr[self.w & m] = arr;
        }
        self.val[self.w & m] = v;
        self.w += 1;
        if self.w - self.r > CAP - 4 {
            self.r = self.w - (CAP - 4);
        }
        cnt
    }
    /// Value arriving at the far end at time t.
    #[inline]
    fn read(&mut self, t: f64) -> f64 {
        let m = CAP - 1;
        if self.w == 0 || self.arr[self.r & m] > t {
            return if self.r == 0 { self.init } else { self.val[self.r & m] };
        }
        while self.r + 1 < self.w && self.arr[(self.r + 1) & m] <= t {
            self.r += 1;
        }
        let v0 = self.val[self.r & m];
        if self.r + 1 < self.w {
            let (a0, a1) = (self.arr[self.r & m], self.arr[(self.r + 1) & m]);
            let d = a1 - a0;
            if d > 1e-12 {
                return v0 + (self.val[(self.r + 1) & m] - v0) * ((t - a0) / d);
            }
        }
        v0
    }
    /// Mean of the arriving signal over [t0, t1] (straight lines between samples, steps where
    /// samples arrive together). Sampling a front through this box, instead of at one instant,
    /// band-limits it: a step between two read times comes out as a one-sample ramp whose height
    /// split says where the step fell, rather than as a full step at the next read time.
    #[inline]
    fn read_avg(&mut self, t0: f64, t1: f64) -> f64 {
        let m = CAP - 1;
        if self.w == 0 || self.arr[self.r & m] >= t1 {
            return if self.r == 0 { self.init } else { self.val[self.r & m] };
        }
        while self.r + 1 < self.w && self.arr[(self.r + 1) & m] <= t0 {
            self.r += 1;
        }
        // before the first sample of the window the signal holds its value
        let mut k = self.r;
        let mut acc = 0.0;
        let mut x = t0;
        let a_first = self.arr[k & m];
        if a_first > t0 {
            let v = if k == 0 { self.init } else { self.val[k & m] };
            acc += v * (a_first - t0);
            x = a_first;
        }
        while k + 1 < self.w {
            let (a0, a1) = (self.arr[k & m], self.arr[(k + 1) & m]);
            if a0 >= t1 {
                break;
            }
            let (v0, v1) = (self.val[k & m], self.val[(k + 1) & m]);
            let lo = x.max(a0);
            let hi = t1.min(a1);
            if hi > lo {
                let d = a1 - a0;
                let f0 = (lo - a0) / d;
                let f1 = (hi - a0) / d;
                acc += (hi - lo) * (v0 + (v1 - v0) * 0.5 * (f0 + f1));
                x = hi;
            }
            if a1 >= t1 {
                break;
            }
            k += 1;
        }
        if x < t1 {
            // past the newest sample: it holds
            acc += self.val[k & m] * (t1 - x);
        }
        acc / (t1 - t0)
    }
    /// Value that will arrive at time t (t may lie ahead of now); does not move the reader.
    fn peek(&self, t: f64) -> f64 {
        let m = CAP - 1;
        if self.w == 0 {
            return self.init;
        }
        let (mut lo, mut hi) = (self.r, self.w - 1);
        if self.arr[lo & m] > t {
            return if self.r == 0 { self.init } else { self.val[lo & m] };
        }
        while hi > lo + 1 {
            let mid = (lo + hi) / 2;
            if self.arr[mid & m] <= t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let v0 = self.val[lo & m];
        if hi > lo && self.arr[hi & m] > t {
            let d = self.arr[hi & m] - self.arr[lo & m];
            if d > 1e-12 {
                return v0 + (self.val[hi & m] - v0) * ((t - self.arr[lo & m]) / d);
            }
        } else if hi > lo {
            return self.val[hi & m];
        }
        v0
    }
}

struct Seg {
    len: f64,
    area: f64,
    dia: f64,
    fric: f64,
    /// flow-loss coefficient at this segment's downstream end, on the dynamic head of the
    /// narrower pipe there (0 = only the unavoidable expansion loss)
    zeta: f64,
    /// absorptive lining: corner frequency of the low-pass each wave sees crossing this
    /// segment (0 = none)
    absorb_hz: f64,
    absorb: f64,
    /// one-pole coefficient of the absorption low-pass, 1 - exp(-2 pi absorb_hz dt), for `al_dt`
    al: f64,
    al_dt: f64,
    lp_r: f64,
    lp_l: f64,
    /// treble loss of each direction's crossing, and its delay at low frequency (s)
    tl_r: Lp2,
    tl_l: Lp2,
    tl_delay: f64,
    /// band-limited reading: previous box mean of each direction (for the triangle)
    bl_r: f64,
    bl_l: f64,
    /// boundary-layer loss of each direction's crossing, and its coefficient a (s^0.5)
    kh_r: Kirch,
    kh_l: Kirch,
    kh_a: f64,
    x0: f64,
    right: Line, // lambda, emitted at the left end
    left: Line,  // beta, emitted at the right end
    lam_in: f64,
    bet_in: f64,
    lam_out: f64, // arriving at the right end
    bet_out: f64, // arriving at the left end
}

pub struct WavePipe {
    segs: Vec<Seg>,
    time: f64,
    pub t_ref: f64,
    t_wall: f64,
    volume: f64,
    pub len: f64,
    /// scratch for the invariants launched this step (kept to avoid two allocations per step)
    lam_new: Vec<f64>,
    bet_new: Vec<f64>,
    /// time step the treble-loss filters were set up for (0 = not yet)
    tl_dt: f64,
    treble_hz: f64,
    band_limit: u8,
    kirchhoff: bool,
    /// temperature the boundary-layer coefficients were last set for
    kh_t: f64,
    /// debugging: per-step trace of every segment's invariants and pooling (ENGSIM_WAVE_TRACE)
    pub trace: Option<std::io::BufWriter<std::fs::File>>,
}

#[inline]
fn interior(inv: f64, a_ref: f64) -> Prim {
    // a state at rest on the arriving characteristic, at the pipe's entropy
    let a = a_ref * inv;
    let p = P_ATM * pow_2g_gm1(inv);
    Prim { rho: GAMMA * p / (a * a), u: 0.0, p }
}

#[inline]
fn amp(p: f64) -> f64 {
    pow_gm1_2g(p / P_ATM)
}

impl WavePipe {
    pub fn new(secs: &[Sec], t_init: f64, t_wall: f64) -> Self {
        Self::new_with(secs, t_init, t_wall, MUFFLER_ZETA)
    }

    /// As `new`, with a muffler exit loss coefficient other than the default.
    pub fn new_with(secs: &[Sec], t_init: f64, t_wall: f64, muffler_zeta: f64) -> Self {
        let mut segs = Vec::new();
        let mut x0 = 0.0;
        let mut add = |len: f64, d: f64, fric: f64, zeta: f64, x0: &mut f64| {
            segs.push(Seg { len, area: std::f64::consts::FRAC_PI_4 * d * d, dia: d, fric, zeta, absorb_hz: if fric >= 0.1 { absorb_hz() } else { WALL_LOSS_HZ }, absorb: if fric >= 0.1 { absorb_frac() } else { 1.0 - (-wall_loss() * len / (0.5 * d)).exp() }, al: 0.0, al_dt: f64::NAN, lp_r: 1.0, lp_l: 1.0, tl_r: Lp2::default(), tl_l: Lp2::default(), tl_delay: 0.0, bl_r: 1.0, bl_l: 1.0, kh_r: Kirch::default(), kh_l: Kirch::default(), kh_a: 0.0, x0: *x0, right: Line::new(1.0), left: Line::new(1.0), lam_in: 1.0, bet_in: 1.0, lam_out: 1.0, bet_out: 1.0 });
            *x0 += len;
        };
        for s in secs {
            let ratio = (s.d1 / s.d0).max(s.d0 / s.d1);
            let parts = if ratio > 1.15 { 4 } else { 1 };
            for j in 0..parts {
                let f = (j as f64 + 0.5) / parts as f64;
                // a section flagged as a muffler (high wall friction) gets the exit restriction
                let zeta = if j == parts - 1 && s.fric >= 0.1 { muffler_zeta } else { 0.0 };
                add(s.len / parts as f64, s.d0 + (s.d1 - s.d0) * f, s.fric, zeta, &mut x0);
            }
        }
        let volume = segs.iter().map(|s| s.len * s.area).sum();
        let ns = segs.len();
        WavePipe { segs, time: 0.0, t_ref: t_init, t_wall, volume, len: x0, lam_new: vec![0.0; ns], bet_new: vec![0.0; ns], tl_dt: 0.0, treble_hz: treble_hz(), band_limit: band_limit(), kirchhoff: kirchhoff_on(), kh_t: 0.0, trace: None }
    }

    /// One time step. Returns the end fluxes (left, right) and the end states (left, right).
    pub fn step(&mut self, dt: f64, bl: Bc, br: Bc, rad_l: &mut Rad, rad_r: &mut Rad) -> (EndFlux, EndFlux, Prim, Prim) {
        self.time += dt;
        let t = self.time;
        let a_ref = (GAMMA * R_GAS * self.t_ref).sqrt();
        let ns = self.segs.len();
        prof::count(cnt::WAVE_SEG_STEPS, ns as u64);
        if self.tl_dt != dt {
            self.tl_dt = dt;
            let on = self.treble_hz > 0.0;
            for s in self.segs.iter_mut() {
                // cutoff such that the loss in nepers grows with the length crossed; capped below
                // Nyquist (only very short pieces get there, where the loss is negligible anyway)
                let fc = if on { (self.treble_hz * s.len.powf(-0.25)).min(0.4 / dt) } else { 0.4 / dt };
                s.tl_r = Lp2::new(fc, dt, s.lam_out);
                s.tl_l = Lp2::new(fc, dt, s.bet_out);
                // a Butterworth pair delays low frequencies by sqrt(2) / (2 pi fc); read that much
                // earlier so the pipe keeps its length
                s.tl_delay = if on { std::f64::consts::SQRT_2 / (2.0 * std::f64::consts::PI * fc) } else { 0.0 };
                s.kh_r = Kirch::new(dt, s.lam_out);
                s.kh_l = Kirch::new(dt, s.bet_out);
                if self.kirchhoff && s.fric < 0.1 {
                    // the boundary-layer loss takes the place of the flat wall-loss shelf
                    s.absorb_hz = 0.0;
                }
            }
            self.kh_t = 0.0;
        }
        if self.kirchhoff && (self.t_ref - self.kh_t).abs() > 0.02 * self.kh_t {
            self.kh_t = self.t_ref;
            let nu = nu_air(self.t_ref);
            for s in self.segs.iter_mut() {
                s.kh_a = s.len * (1.0 + GM1 / PRANDTL.sqrt()) * nu.sqrt() / (0.5 * s.dia * a_ref);
            }
        }
        let bl_mode = self.band_limit;
        // arrivals, with treble loss, and friction lumped over the transit
        for s in self.segs.iter_mut() {
            // read ahead by the filter's delay, so the pipe keeps its acoustic length
            let ta = t + s.tl_delay;
            let (mut lam, mut bet) = match bl_mode {
                0 => (s.right.read(ta), s.left.read(ta)),
                1 => (s.right.read_avg(ta - 0.5 * dt, ta + 0.5 * dt), s.left.read_avg(ta - 0.5 * dt, ta + 0.5 * dt)),
                _ => {
                    // triangle centred on ta: mean of this period's box and the last one's
                    let (br, bl_) = (s.right.read_avg(ta, ta + dt), s.left.read_avg(ta, ta + dt));
                    let v = (0.5 * (br + s.bl_r), 0.5 * (bl_ + s.bl_l));
                    s.bl_r = br;
                    s.bl_l = bl_;
                    v
                }
            };
            if self.kirchhoff {
                lam = s.kh_r.run(lam, s.kh_a);
                bet = s.kh_l.run(bet, s.kh_a);
            }
            if s.tl_delay > 0.0 {
                lam = s.tl_r.run(lam);
                bet = s.tl_l.run(bet);
            }
            if s.absorb_hz > 0.0 {
                // (the coefficient depends only on dt, which is fixed for a run; same value as computing it every step)
                if dt != s.al_dt {
                    s.al = 1.0 - (-2.0 * std::f64::consts::PI * s.absorb_hz * dt).exp();
                    s.al_dt = dt;
                }
                let al = s.al;
                s.lp_r += al * (lam - s.lp_r);
                s.lp_l += al * (bet - s.lp_l);
                // a shelf: frequencies above the corner lose a fixed fraction per crossing
                lam -= s.absorb * (lam - s.lp_r);
                bet -= s.absorb * (bet - s.lp_l);
            }
            // friction acts on each wave sample through its own particle velocity (riding on the
            // opposite wave's mean), so sound is damped in proportion to the mean flow
            let tau = s.len / (a_ref * 0.5 * (lam + bet)).max(50.0);
            let g = K * s.fric / (2.0 * s.dia) * tau * a_ref / (GM1 * GM1);
            let ul = lam - 0.5 * (s.bet_in + s.bet_out);
            let ub = 0.5 * (s.lam_in + s.lam_out) - bet;
            s.lam_out = lam - g * ul * ul.abs();
            s.bet_out = bet + g * ub * ub.abs();
        }
        // ends
        let end = |inv: f64, bc: Bc, rad: &mut Rad, area: f64| -> (Prim, f64) {
            // returns the face state (u along the outward normal) and the reflected invariant
            match bc {
                Bc::Closed => {
                    let a = a_ref * inv;
                    let p = P_ATM * pow_2g_gm1(inv);
                    (Prim { rho: GAMMA * p / (a * a), u: 0.0, p }, inv)
                }
                Bc::Res { p0, rho0 } => {
                    let f = reservoir_face(interior(inv, a_ref), p0, rho0);
                    (f, amp(f.p) + K * f.u / a_ref)
                }
                Bc::Open => {
                    let f = rad.solve(interior(inv, a_ref), area, dt);
                    (f, amp(f.p) + K * f.u / a_ref)
                }
            }
        };
        let (fl, lam0) = end(self.segs[0].bet_out, bl, rad_l, self.segs[0].area);
        let (fr, betn) = end(self.segs[ns - 1].lam_out, br, rad_r, self.segs[ns - 1].area);
        // the invariant sent back into the pipe: A - K u_n / a_ref (flow out of the pipe lowers it)
        let lam0 = 2.0 * amp(fl.p) - lam0;
        let betn = 2.0 * amp(fr.p) - betn;
        let pt = prof::now();
        // junctions: equal pressure, continuous volume flow
        let mut lam_new = std::mem::take(&mut self.lam_new);
        let mut bet_new = std::mem::take(&mut self.bet_new);
        lam_new[0] = lam0;
        bet_new[ns - 1] = betn;
        for j in 0..ns - 1 {
            // Area step. Volume flow is continuous (U2 = r U1). Pressure across the step follows
            // Bernoulli where the flow speeds up into a smaller pipe, and the sudden-expansion
            // (Borda-Carnot) momentum balance where it dumps into a larger one; that loss is what
            // lets a muffler absorb sound when gas is flowing. Low-Mach forms, solved by iteration
            // from the lossless answer.
            let (a1, a2) = (self.segs[j].area, self.segs[j + 1].area);
            let (l1, b2) = (self.segs[j].lam_out, self.segs[j + 1].bet_out);
            let r = a1 / a2;
            let abar = 0.5 * (l1 + b2);
            let lin = (l1 - b2) / (K * (1.0 + r));
            // quadratic in U1: c2 U1^2 - U1 + lin = 0, taking the root that tends to `lin`
            let fwd = lin > 0.0;
            let w = if fwd && r < 1.0 {
                r
            } else if !fwd && r > 1.0 {
                1.0
            } else {
                0.5 * (1.0 + r)
            };
            let sref = if r < 1.0 { 1.0 } else { r };
            let loss = 0.5 * self.segs[j].zeta * sref * sref;
            let c2 = ((1.0 - r) * w - if fwd { loss } else { -loss }) / (abar * (1.0 + r));
            let u1 = 2.0 * lin / (1.0 + (1.0 - 4.0 * c2 * lin).max(0.0).sqrt());
            bet_new[j] = l1 - 2.0 * K * u1;
            lam_new[j + 1] = b2 + 2.0 * K * r * u1;
        }
        // launch, each sample at its own speed
        let c1 = (GAMMA + 1.0) / (2.0 * GM1);
        let c2 = (3.0 - GAMMA) / (2.0 * GM1);
        let mut heat = 0.0;
        let mut tr: Vec<f64> = Vec::new();
        for (j, s) in self.segs.iter_mut().enumerate() {
            let (lam, bet) = (lam_new[j], bet_new[j]);
            let bet_mean = 0.5 * (s.bet_out + bet);
            let lam_mean = 0.5 * (s.lam_out + lam);
            let cr = (a_ref * (c1 * lam - c2 * bet_mean)).max(0.08 * a_ref);
            let cl = (a_ref * (c1 * bet - c2 * lam_mean)).max(0.08 * a_ref);
            let pr = s.right.push(lam, t + s.len / cr, t, t + s.tl_delay + dt);
            let pl = s.left.push(bet, t + s.len / cl, t, t + s.tl_delay + dt);
            if self.trace.is_some() {
                tr.extend_from_slice(&[s.lam_out, s.bet_out, lam, bet, pr, pl]);
            }
            s.lam_in = lam;
            s.bet_in = bet;
            // wall heat, lumped
            let u = (a_ref * (lam - s.bet_out) / GM1).abs();
            let rho = P_ATM / (R_GAS * self.t_ref);
            let hc = 0.125 * s.fric * rho * u * CP + 8.0;
            heat += hc * 4.0 / (s.dia * rho * CV) * (s.len * s.area);
        }
        self.lam_new = lam_new;
        self.bet_new = bet_new;
        prof::lap(cat::WAVE_JUNCTION, pt);
        // The wall acts on the gas's actual temperature, T_ref * A^2 at the pipe's mean pressure
        // amplitude A, not on the reference temperature (they differ a lot under manifold vacuum).
        let mut t_ref = self.t_ref;
        let a_mean = 0.25 * (self.segs[0].lam_in + self.segs[0].bet_out + self.segs[ns - 1].lam_out + self.segs[ns - 1].bet_in);
        let target = self.t_wall / (a_mean * a_mean).max(0.05);
        let x = heat / self.volume * dt;
        t_ref = target + (t_ref - target) / (1.0 + x + 0.5 * x * x);
        // gas entering resets the pipe's entropy level
        let m_pipe = P_ATM / (R_GAS * t_ref) * self.volume;
        for (f, area) in [(fl, self.segs[0].area), (fr, self.segs[ns - 1].area)] {
            if f.u < 0.0 {
                let m_in = -f.rho * f.u * area * dt;
                let t_in = f.p / (f.rho * R_GAS) * pow_gm1_g(P_ATM / f.p);
                t_ref += (m_in / m_pipe).min(0.5) * (t_in - t_ref);
            }
        }
        self.t_ref = t_ref.clamp(250.0, 2500.0);
        if let Some(w) = self.trace.as_mut() {
            use std::io::Write;
            tr.extend_from_slice(&[t, fr.p, fr.u, fr.u * self.segs[ns - 1].area, self.t_ref]);
            for v in &tr {
                let _ = w.write_all(&v.to_le_bytes());
            }
        }
        let flux = |f: Prim, area: f64| {
            let g = f.flux();
            EndFlux { mdot: g[0] * area, edot: g[2] * area, q: f.u * area }
        };
        // end states with u along +x
        let sl = Prim { rho: fl.rho, u: -fl.u, p: fl.p };
        (flux(fl, self.segs[0].area), flux(fr, self.segs[ns - 1].area), sl, fr)
    }

    /// Gas state at distance x from the left end (for pictures; not used by the solver).
    pub fn state_at(&self, x: f64) -> Prim {
        let a_ref = (GAMMA * R_GAS * self.t_ref).sqrt();
        let mut seg = &self.segs[self.segs.len() - 1];
        for s in &self.segs {
            if x < s.x0 + s.len {
                seg = s;
                break;
            }
        }
        let xl = (x - seg.x0).clamp(0.0, seg.len);
        let c = a_ref * 0.5 * (seg.lam_in + seg.bet_in);
        let lam = seg.right.peek(self.time + (seg.len - xl) / c);
        let bet = seg.left.peek(self.time + xl / c);
        let a = 0.5 * (lam + bet);
        let p = P_ATM * pow_2g_gm1(a);
        let aa = a_ref * a;
        Prim { rho: GAMMA * p / (aa * aa), u: a_ref * (lam - bet) / GM1, p }
    }
}
