//! Quasi-1D compressible flow in a pipe of varying cross-section.
//! Finite volume, MUSCL-Hancock reconstruction, HLLC fluxes, wall friction and heat transfer.

use crate::gas::*;
use crate::prof::{self, cat, cnt};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum End {
    Closed,
    Volume(usize),
    Ambient,
}

#[derive(Clone, Copy)]
pub enum Bc {
    Closed,
    /// Open to free air through the pipe's own radiation load.
    Open,
    Res { p0: f64, rho0: f64 },
}

#[derive(Clone, Copy, Default)]
pub struct EndFlux {
    /// Mass rate leaving the pipe through this end (kg/s).
    pub mdot: f64,
    /// Total enthalpy rate leaving (W).
    pub edot: f64,
    /// Volume flow leaving (m^3/s).
    pub q: f64,
}

/// One straight or tapered stretch of a pipe.
#[derive(Clone, Copy)]
pub struct Sec {
    pub len: f64,
    pub d0: f64,
    pub d1: f64,
    pub fric: f64,
}

/// Flanagan-style parallel R-L radiation load on an open end.
#[derive(Clone, Copy, Default)]
pub struct Rad {
    pub r: f64,
    pub l: f64,
    pub u_l: f64,
    pub p: f64,
    /// experiment (`+r` quality): start the secant search with the slope the last solve ended on
    pub fast: bool,
    slope: f64,
    p_prev: f64,
}

impl Rad {
    pub fn new(area: f64) -> Self {
        let r = 128.0 * RHO_ATM * C_ATM / (9.0 * std::f64::consts::PI.powi(2) * area);
        let l = 8.0 * RHO_ATM / (3.0 * std::f64::consts::PI * (std::f64::consts::PI * area).sqrt());
        Rad { r, l, u_l: 0.0, p: 0.0, fast: false, slope: 2.0, p_prev: 0.0 }
    }
    /// Face state of an open end radiating into free air. The radiation pressure
    /// p = R (Q - U_L) and the face flow Q(p) are solved together. Q falls as p rises, so the
    /// residual is monotone: a secant search started from the last step's answer converges in
    /// two or three evaluations, with a bracketed search as the fallback.
    /// `i` is the interior state with u along the outward normal.
    #[inline]
    pub fn solve(&mut self, i: Prim, area: f64, dt: f64) -> Prim {
        let pt = prof::now();
        prof::count(cnt::RAD_CALLS, 1);
        let (r, u_l) = (self.r, self.u_l);
        let g = |pr: f64| {
            prof::count(cnt::RAD_EVALS, 1);
            let f = reservoir_face(i, P_ATM + pr, RHO_ATM);
            (pr - r * (area * f.u - u_l), f)
        };
        const TOL: f64 = 1e-7;
        // (`fast`: extrapolate the last two answers for the first guess)
        let mut x0 = if self.fast { 2.0 * self.p - self.p_prev } else { self.p };
        let (mut g0, mut face) = g(x0);
        if prof::ON && g0.abs() <= TOL {
            prof::count(cnt::RAD_ZERO_ITER, 1);
        }
        if g0.abs() > TOL {
            // dg/dp is at least 1
            let mut x1 = if self.fast { x0 - g0 / self.slope } else { x0 - 0.5 * g0 };
            let mut done = false;
            for _ in 0..6 {
                let (g1, f1) = g(x1);
                face = f1;
                if g1.abs() <= TOL {
                    if self.fast {
                        let k = (g1 - g0) / (x1 - x0);
                        if k.is_finite() && k >= 1.0 {
                            self.slope = k;
                        }
                    }
                    x0 = x1;
                    done = true;
                    break;
                }
                let den = g1 - g0;
                if den == 0.0 || !den.is_finite() {
                    break;
                }
                let x2 = x1 - g1 * (x1 - x0) / den;
                x0 = x1;
                g0 = g1;
                x1 = x2;
                if !x1.is_finite() || x1 < -0.8 * P_ATM {
                    break;
                }
            }
            if !done {
                prof::count(cnt::RAD_FALLBACK, 1);
                // bracketed fallback (regula falsi, Illinois variant)
                let (mut lo, mut hi) = (self.p - 2000.0, self.p + 2000.0);
                let mut glo = g(lo).0;
                let mut ghi = g(hi).0;
                let mut guard = 0;
                while glo > 0.0 && guard < 8 {
                    lo -= (hi - lo) * 2.0;
                    glo = g(lo).0;
                    guard += 1;
                }
                while ghi < 0.0 && guard < 16 {
                    hi += (hi - lo) * 2.0;
                    ghi = g(hi).0;
                    guard += 1;
                }
                lo = lo.max(-0.8 * P_ATM);
                for _ in 0..60 {
                    x0 = if glo * ghi < 0.0 { (lo * ghi - hi * glo) / (ghi - glo) } else { 0.5 * (lo + hi) };
                    let (gm, f) = g(x0);
                    face = f;
                    if gm.abs() <= TOL || (hi - lo) < 1e-9 {
                        break;
                    }
                    if gm < 0.0 {
                        lo = x0;
                        glo = gm;
                        ghi *= 0.5;
                    } else {
                        hi = x0;
                        ghi = gm;
                        glo *= 0.5;
                    }
                }
            }
        }
        self.p_prev = self.p;
        self.p = x0;
        self.u_l += dt * x0 / self.l;
        prof::lap(cat::RAD, pt);
        face
    }
}

pub struct Pipe {
    pub name: String,
    pub kind: String,
    pub cyl: i32,
    pub group: i32,
    pub n: usize,
    pub dx: f64,
    pub len: f64,
    pub af: Vec<f64>,
    pub vol: Vec<f64>,
    pub dia: Vec<f64>,
    pub fric: Vec<f64>,
    pub rho: Vec<f64>,
    pub mom: Vec<f64>,
    pub ene: Vec<f64>,
    pub left: End,
    pub right: End,
    pub t_wall: f64,
    pub rad_l: Rad,
    pub rad_r: Rad,
    pub out_l: EndFlux,
    pub out_r: EndFlux,
    /// profiling: when on, nanoseconds spent in [primitives, reconstruction, fluxes, update+sources]
    pub prof_on: bool,
    pub prof: [u64; 4],
    /// largest wave speed this pipe has seen (m/s)
    /// 0 = second-order (van Leer) slopes, 1 = third-order (Koren) face values
    pub recon: u8,
    /// experiment (`+f` quality): reconstruction and fluxes in f32, state and update in f64
    pub f32: bool,
    s32: Option<Box<Scratch32>>,
    /// when set, the pipe is solved along characteristics and the cells are only a display copy
    pub wave: Option<Box<crate::wave::WavePipe>>,
    /// muffler exit loss coefficient for this pipe on characteristics (None: the default)
    pub muffler_zeta: Option<f64>,
    secs: Vec<Sec>,
    t_init: f64,
    pub smax_seen: f64,
    pub cfl_last: f64,
    pub dt_last: f64,
    pub substeps: u64,
    // scratch, structure-of-arrays so the loops vectorize
    pr: Vec<f64>,
    pu: Vec<f64>,
    pp: Vec<f64>,
    ir: Vec<f64>,
    lr: Vec<f64>,
    lu: Vec<f64>,
    lp: Vec<f64>,
    rr: Vec<f64>,
    ru: Vec<f64>,
    rp: Vec<f64>,
    f0: Vec<f64>,
    f1: Vec<f64>,
    f2: Vec<f64>,
    sp: Vec<f64>,
    inv_vol: Vec<f64>,
    fric_2d: Vec<f64>,
    four_d: Vec<f64>,
}

impl Pipe {
    pub fn new(name: &str, kind: &str, secs: &[Sec], dx_target: f64, left: End, right: End, t_init: f64, t_wall: f64) -> Self {
        let len: f64 = secs.iter().map(|s| s.len).sum();
        let n = ((len / dx_target).round() as usize).max(4);
        let dx = len / n as f64;
        let mut dia = vec![0.0; n];
        let mut fric = vec![0.0; n];
        for i in 0..n {
            let x = (i as f64 + 0.5) * dx;
            let mut x0 = 0.0;
            let mut found = false;
            for s in secs {
                if x < x0 + s.len {
                    let f = (x - x0) / s.len;
                    dia[i] = s.d0 + (s.d1 - s.d0) * f;
                    fric[i] = s.fric;
                    found = true;
                    break;
                }
                x0 += s.len;
            }
            if !found {
                let s = secs.last().unwrap();
                dia[i] = s.d1;
                fric[i] = s.fric;
            }
        }
        let area: Vec<f64> = dia.iter().map(|d| std::f64::consts::FRAC_PI_4 * d * d).collect();
        let mut af = vec![0.0; n + 1];
        af[0] = area[0];
        af[n] = area[n - 1];
        for i in 1..n {
            af[i] = area[i - 1].min(area[i]);
        }
        let vol: Vec<f64> = area.iter().map(|a| a * dx).collect();
        let rho0 = P_ATM / (R_GAS * t_init);
        let inv_vol: Vec<f64> = vol.iter().map(|v| 1.0 / v).collect();
        let fric_2d: Vec<f64> = fric.iter().zip(&dia).map(|(f, d)| f / (2.0 * d)).collect();
        let four_d: Vec<f64> = dia.iter().map(|d| 4.0 / d).collect();
        Pipe {
            name: name.to_string(),
            kind: kind.to_string(),
            cyl: -1,
            group: -1,
            n,
            dx,
            len,
            rad_l: Rad::new(af[0]),
            rad_r: Rad::new(af[n]),
            af,
            vol,
            dia,
            fric,
            rho: vec![rho0; n],
            mom: vec![0.0; n],
            ene: vec![P_ATM / GM1; n],
            left,
            right,
            t_wall,
            out_l: EndFlux::default(),
            out_r: EndFlux::default(),
            prof_on: false,
            prof: [0; 4],
            recon: 0,
            f32: false,
            s32: None,
            wave: None,
            muffler_zeta: None,
            secs: secs.to_vec(),
            t_init,
            smax_seen: 0.0,
            cfl_last: 0.5,
            dt_last: 1.0,
            substeps: 0,
            pr: vec![0.0; n],
            pu: vec![0.0; n],
            pp: vec![0.0; n],
            ir: vec![0.0; n],
            lr: vec![0.0; n],
            lu: vec![0.0; n],
            lp: vec![0.0; n],
            rr: vec![0.0; n],
            ru: vec![0.0; n],
            rp: vec![0.0; n],
            f0: vec![0.0; n + 1],
            f1: vec![0.0; n + 1],
            f2: vec![0.0; n + 1],
            sp: vec![0.0; n + 1],
            inv_vol,
            fric_2d,
            four_d,
        }
    }

    /// Switch this pipe to the characteristic (delay-line) solver.
    pub fn make_wave(&mut self) {
        let mut wp = Box::new(match self.muffler_zeta {
            Some(z) => crate::wave::WavePipe::new_with(&self.secs, self.t_init, self.t_wall, z),
            None => crate::wave::WavePipe::new(&self.secs, self.t_init, self.t_wall),
        });
        if std::env::var("ENGSIM_WAVE_TRACE").map(|v| v == self.name).unwrap_or(false) {
            let path = std::env::var("ENGSIM_WAVE_TRACE_FILE").unwrap_or_else(|_| "wave_trace.bin".into());
            wp.trace = Some(std::io::BufWriter::new(std::fs::File::create(path).unwrap()));
        }
        self.wave = Some(wp);
    }

    fn set_cell(&mut self, i: usize, w: Prim) {
        self.rho[i] = w.rho;
        self.mom[i] = w.rho * w.u;
        self.ene[i] = w.energy();
    }

    /// Fill the display cells from the characteristic solver (no-op for grid pipes).
    pub fn refresh_cells(&mut self) {
        if let Some(wp) = self.wave.take() {
            for i in 0..self.n {
                let w = wp.state_at((i as f64 + 0.5) * self.dx);
                self.set_cell(i, w);
            }
            self.wave = Some(wp);
        }
    }

    #[inline]
    pub fn prim(&self, i: usize) -> Prim {
        let rho = self.rho[i];
        let u = self.mom[i] / rho;
        Prim { rho, u, p: GM1 * (self.ene[i] - 0.5 * rho * u * u) }
    }

    pub fn mass(&self) -> f64 {
        (0..self.n).map(|i| self.rho[i] * self.vol[i]).sum()
    }

    /// Advance one step. Returns the largest wave speed seen (for the CFL check).
    /// The four passes are written branch-free over plain slices so the compiler vectorizes them.
    pub fn step(&mut self, dt: f64, bl: Bc, br: Bc) -> f64 {
        let n = self.n;
        if let Some(mut wp) = self.wave.take() {
            let pt = prof::now();
            let (ol, or, sl, sr) = wp.step(dt, bl, br, &mut self.rad_l, &mut self.rad_r);
            self.out_l = ol;
            self.out_r = or;
            self.set_cell(0, sl);
            self.set_cell(n - 1, sr);
            self.wave = Some(wp);
            prof::lap(cat::WAVE, pt);
            return 0.0;
        }
        let t0 = if self.prof_on { Some(std::time::Instant::now()) } else { None };
        let pt = prof::now();
        prof::count(cnt::GRID_CELL_STEPS, n as u64);
        // 1. primitives
        {
            let (rho, mom, ene) = (&self.rho[..n], &self.mom[..n], &self.ene[..n]);
            let (pr, pu, pp, ir) = (&mut self.pr[..n], &mut self.pu[..n], &mut self.pp[..n], &mut self.ir[..n]);
            for i in 0..n {
                let r = rho[i];
                let inv = 1.0 / r;
                let u = mom[i] * inv;
                pr[i] = r;
                pu[i] = u;
                ir[i] = inv;
                pp[i] = GM1 * (ene[i] - 0.5 * r * u * u);
            }
        }
        let pt = prof::lap(cat::GRID_PRIM, pt);
        let t1 = t0.map(|_| std::time::Instant::now());
        let (pt, t2) = if self.f32 {
            self.recon_flux_f32(dt);
            (pt, t1)
        } else {
            // 2. limited slopes and the half-step predictor (MUSCL-Hancock); end cells stay first order
            {
                let k = 0.5 * dt / self.dx;
                let m = n - 2;
                let (r0, r1, r2) = (&self.pr[..m], &self.pr[1..m + 1], &self.pr[2..m + 2]);
                let (u0, u1, u2) = (&self.pu[..m], &self.pu[1..m + 1], &self.pu[2..m + 2]);
                let (p0, p1, p2) = (&self.pp[..m], &self.pp[1..m + 1], &self.pp[2..m + 2]);
                let ir1 = &self.ir[1..m + 1];
                let (lr, lu, lp) = (&mut self.lr[1..m + 1], &mut self.lu[1..m + 1], &mut self.lp[1..m + 1]);
                let (rr, ru, rp) = (&mut self.rr[1..m + 1], &mut self.ru[1..m + 1], &mut self.rp[1..m + 1]);
                #[inline(always)]
                fn lim(a: f64, b: f64) -> f64 {
                    let pr = a * b;
                    let v = 2.0 * pr / (a + b);
                    if pr > 0.0 {
                        v
                    } else {
                        0.0
                    }
                }
                if self.recon == 1 {
                    // Third-order upwind-biased face values (kappa = 1/3) with Koren's limiter:
                    // fewer cells per wavelength for the same accuracy, and no divisions.
                    #[inline(always)]
                    fn k3(a: f64, b: f64) -> f64 {
                        let m = (2.0 * a).abs().min((2.0 * b).abs()).min(((a + 2.0 * b) * (1.0 / 3.0)).abs());
                        if a * b > 0.0 {
                            m.copysign(a)
                        } else {
                            0.0
                        }
                    }
                    for i in 0..m {
                        let (cr, cu, cp) = (r1[i], u1[i], p1[i]);
                        let (ar, br) = (cr - r0[i], r2[i] - cr);
                        let (au, bu) = (cu - u0[i], u2[i] - cu);
                        let (ap, bp) = (cp - p0[i], p2[i] - cp);
                        let (srr, slr) = (k3(ar, br), k3(br, ar));
                        let (sru, slu) = (k3(au, bu), k3(bu, au));
                        let (srp, slp) = (k3(ap, bp), k3(bp, ap));
                        let (dr, du, dp) = (0.5 * (srr + slr), 0.5 * (sru + slu), 0.5 * (srp + slp));
                        let r_ = cr - k * (cu * dr + cr * du);
                        let u_ = cu - k * (cu * du + dp * ir1[i]);
                        let p_ = cp - k * (cu * dp + GAMMA * cp * du);
                        let (a_r, a_p) = (r_ - 0.5 * slr, p_ - 0.5 * slp);
                        let (b_r, b_p) = (r_ + 0.5 * srr, p_ + 0.5 * srp);
                        let ok = (a_r > 0.0) & (a_p > 0.0) & (b_r > 0.0) & (b_p > 0.0);
                        lr[i] = if ok { a_r } else { cr };
                        lu[i] = if ok { u_ - 0.5 * slu } else { cu };
                        lp[i] = if ok { a_p } else { cp };
                        rr[i] = if ok { b_r } else { cr };
                        ru[i] = if ok { u_ + 0.5 * sru } else { cu };
                        rp[i] = if ok { b_p } else { cp };
                    }
                } else {
                    for i in 0..m {
                        let (cr, cu, cp) = (r1[i], u1[i], p1[i]);
                        let dr = lim(cr - r0[i], r2[i] - cr);
                        let du = lim(cu - u0[i], u2[i] - cu);
                        let dp = lim(cp - p0[i], p2[i] - cp);
                        let r_ = cr - k * (cu * dr + cr * du);
                        let u_ = cu - k * (cu * du + dp * ir1[i]);
                        let p_ = cp - k * (cu * dp + GAMMA * cp * du);
                        let (a_r, a_p) = (r_ - 0.5 * dr, p_ - 0.5 * dp);
                        let (b_r, b_p) = (r_ + 0.5 * dr, p_ + 0.5 * dp);
                        let ok = (a_r > 0.0) & (a_p > 0.0) & (b_r > 0.0) & (b_p > 0.0);
                        lr[i] = if ok { a_r } else { cr };
                        lu[i] = if ok { u_ - 0.5 * du } else { cu };
                        lp[i] = if ok { a_p } else { cp };
                        rr[i] = if ok { b_r } else { cr };
                        ru[i] = if ok { u_ + 0.5 * du } else { cu };
                        rp[i] = if ok { b_p } else { cp };
                    }
                }
                for &i in &[0, n - 1] {
                    self.lr[i] = self.pr[i];
                    self.lu[i] = self.pu[i];
                    self.lp[i] = self.pp[i];
                    self.rr[i] = self.pr[i];
                    self.ru[i] = self.pu[i];
                    self.rp[i] = self.pp[i];
                }
            }
            let pt = prof::lap(cat::GRID_RECON, pt);
            let t2 = t0.map(|_| std::time::Instant::now());
            // 3. HLLC fluxes on interior faces (face i sits between cells i-1 and i)
            {
                let m = n - 1;
                let (lr, lu, lp) = (&self.rr[..m], &self.ru[..m], &self.rp[..m]);
                let (rr, ru, rp) = (&self.lr[1..m + 1], &self.lu[1..m + 1], &self.lp[1..m + 1]);
                let (f0, f1, f2, sp) = (&mut self.f0[1..m + 1], &mut self.f1[1..m + 1], &mut self.f2[1..m + 1], &mut self.sp[1..m + 1]);
                let kq = (GAMMA + 1.0) / (2.0 * GAMMA);
                let inv_gm1 = 1.0 / GM1;
                for i in 0..m {
                    let (dl, ul, pl) = (lr[i], lu[i], lp[i]);
                    let (dr, ur, pr) = (rr[i], ru[i], rp[i]);
                    let (idl, idr) = (1.0 / dl, 1.0 / dr);
                    let (ipl, ipr) = (1.0 / pl, 1.0 / pr);
                    let al = (GAMMA * pl * idl).sqrt();
                    let ar = (GAMMA * pr * idr).sqrt();
                    let ps = (0.5 * (pl + pr) - 0.125 * (ur - ul) * (dl + dr) * (al + ar)).max(0.0);
                    let ql = (1.0 + kq * (ps * ipl - 1.0)).max(1.0).sqrt();
                    let qr = (1.0 + kq * (ps * ipr - 1.0)).max(1.0).sqrt();
                    let sl = ul - al * ql;
                    let sr = ur + ar * qr;
                    let ml = dl * (sl - ul);
                    let mr = dr * (sr - ur);
                    let ss = (pr - pl + ul * ml - ur * mr) / (ml - mr);
                    let el = pl * inv_gm1 + 0.5 * dl * ul * ul;
                    let er = pr * inv_gm1 + 0.5 * dr * ur * ur;
                    // pick the side the contact wave leaves us on, then build that star flux
                    let left = ss >= 0.0;
                    let d = if left { dl } else { dr };
                    let u = if left { ul } else { ur };
                    let pq = if left { pl } else { pr };
                    let e = if left { el } else { er };
                    let mm = if left { ml } else { mr };
                    let s = if left { sl } else { sr };
                    let id = if left { idl } else { idr };
                    let sc = if left { sl.min(0.0) } else { sr.max(0.0) };
                    let c = mm / (s - ss);
                    f0[i] = d * u + sc * (c - d);
                    f1[i] = d * u * u + pq + sc * (c * ss - d * u);
                    f2[i] = u * (e + pq) + sc * (c * (e * id + (ss - u) * (ss + pq / mm)) - e);
                    sp[i] = (-sl).max(sr);
                }
            }
            (pt, t2)
        };
        let pt = prof::lap(cat::GRID_FLUX, pt);
        // boundary faces
        let c0 = Prim { rho: self.pr[0], u: self.pu[0], p: self.pp[0] };
        let (fa, q0) = match bl {
            Bc::Closed => (hllc(Prim { rho: c0.rho, u: -c0.u, p: c0.p }, c0), 0.0),
            Bc::Res { p0, rho0 } => {
                let mut f = reservoir_face(Prim { rho: c0.rho, u: -c0.u, p: c0.p }, p0, rho0);
                f.u = -f.u;
                (f.flux(), -f.u * self.af[0])
            }
            Bc::Open => {
                let mut f = self.rad_l.solve(Prim { rho: c0.rho, u: -c0.u, p: c0.p }, self.af[0], dt);
                f.u = -f.u;
                (f.flux(), -f.u * self.af[0])
            }
        };
        let cn = Prim { rho: self.pr[n - 1], u: self.pu[n - 1], p: self.pp[n - 1] };
        let (fb, qn) = match br {
            Bc::Closed => (hllc(cn, Prim { rho: cn.rho, u: -cn.u, p: cn.p }), 0.0),
            Bc::Res { p0, rho0 } => {
                let f = reservoir_face(cn, p0, rho0);
                (f.flux(), f.u * self.af[n])
            }
            Bc::Open => {
                let f = self.rad_r.solve(cn, self.af[n], dt);
                (f.flux(), f.u * self.af[n])
            }
        };
        self.f0[0] = fa[0];
        self.f1[0] = fa[1];
        self.f2[0] = fa[2];
        self.f0[n] = fb[0];
        self.f1[n] = fb[1];
        self.f2[n] = fb[2];
        self.sp[0] = c0.u.abs() + c0.a();
        self.sp[n] = cn.u.abs() + cn.a();
        self.out_l = EndFlux { mdot: -fa[0] * self.af[0], edot: -fa[2] * self.af[0], q: q0 };
        self.out_r = EndFlux { mdot: fb[0] * self.af[n], edot: fb[2] * self.af[n], q: qn };
        let pt = prof::lap(cat::GRID_BC, pt);
        let t3 = t0.map(|_| std::time::Instant::now());
        // 4. conservative update, then wall friction (kinetic energy to heat) and heat transfer to the wall
        {
            let (af0, af1) = (&self.af[..n], &self.af[1..n + 1]);
            let (g0a, g0b) = (&self.f0[..n], &self.f0[1..n + 1]);
            let (g1a, g1b) = (&self.f1[..n], &self.f1[1..n + 1]);
            let (g2a, g2b) = (&self.f2[..n], &self.f2[1..n + 1]);
            let (iv, pp, fr, f2d, fd) = (&self.inv_vol[..n], &self.pp[..n], &self.fric[..n], &self.fric_2d[..n], &self.four_d[..n]);
            let (rho_a, mom_a, ene_a) = (&mut self.rho[..n], &mut self.mom[..n], &mut self.ene[..n]);
            let tw = self.t_wall;
            let inv_cv = 1.0 / CV;
            for i in 0..n {
                let (a0, a1) = (af0[i], af1[i]);
                let inv = dt * iv[i];
                let rho = (rho_a[i] + inv * (a0 * g0a[i] - a1 * g0b[i])).max(1e-4);
                let mom = mom_a[i] + inv * (a0 * g1a[i] - a1 * g1b[i] + pp[i] * (a1 - a0));
                let ene = ene_a[i] + inv * (a0 * g2a[i] - a1 * g2b[i]);
                let irho = 1.0 / rho;
                let u = mom * irho;
                let mut eint = (ene - 0.5 * rho * u * u).max(250.0);
                let un = u / (1.0 + dt * f2d[i] * u.abs());
                eint += 0.5 * rho * (u * u - un * un);
                let rcv = rho * CV;
                let t = eint * irho * inv_cv;
                let hc = 0.125 * fr[i] * rho * un.abs() * CP + 8.0;
                // exp(-x) for the tiny x of one step, to third order
                let x = hc * fd[i] * dt * inv_cv * irho;
                let decay = 1.0 / (1.0 + x + 0.5 * x * x);
                let tn = tw + (t - tw) * decay;
                rho_a[i] = rho;
                mom_a[i] = rho * un;
                ene_a[i] = rcv * tn + 0.5 * rho * un * un;
            }
        }
        let pt = prof::lap(cat::GRID_UPDATE, pt);
        if let (Some(a), Some(b), Some(c), Some(d)) = (t0, t1, t2, t3) {
            let e = std::time::Instant::now();
            self.prof[0] += (b - a).as_nanos() as u64;
            self.prof[1] += (c - b).as_nanos() as u64;
            self.prof[2] += (d - c).as_nanos() as u64;
            self.prof[3] += (e - d).as_nanos() as u64;
        }
        let smax = self.sp[..n + 1].iter().fold(0.0f64, |m, v| if *v > m { *v } else { m });
        if smax > self.smax_seen {
            self.smax_seen = smax;
        }
        prof::lap(cat::GRID_SMAX, pt);
        smax
    }

    /// Steps 2 and 3 of `step` in 32-bit floats: four cells per wasm SIMD instruction instead of
    /// two. Primitives come in from the f64 pass; fluxes and wave speeds go out to the f64
    /// face arrays, so the conservative update and the boundaries stay 64-bit.
    fn recon_flux_f32(&mut self, dt: f64) {
        let n = self.n;
        let sc = self.s32.get_or_insert_with(|| Box::new(Scratch32::new(n)));
        for i in 0..n {
            sc.pr[i] = self.pr[i] as f32;
            sc.pu[i] = self.pu[i] as f32;
            sc.pp[i] = self.pp[i] as f32;
            sc.ir[i] = self.ir[i] as f32;
        }
        let g = GAMMA as f32;
        {
            let k = (0.5 * dt / self.dx) as f32;
            let m = n - 2;
            let (r0, r1, r2) = (&sc.pr[..m], &sc.pr[1..m + 1], &sc.pr[2..m + 2]);
            let (u0, u1, u2) = (&sc.pu[..m], &sc.pu[1..m + 1], &sc.pu[2..m + 2]);
            let (p0, p1, p2) = (&sc.pp[..m], &sc.pp[1..m + 1], &sc.pp[2..m + 2]);
            let ir1 = &sc.ir[1..m + 1];
            let (lr, lu, lp) = (&mut sc.lr[1..m + 1], &mut sc.lu[1..m + 1], &mut sc.lp[1..m + 1]);
            let (rr, ru, rp) = (&mut sc.rr[1..m + 1], &mut sc.ru[1..m + 1], &mut sc.rp[1..m + 1]);
            #[inline(always)]
            fn lim(a: f32, b: f32) -> f32 {
                let pr = a * b;
                let v = 2.0 * pr / (a + b);
                if pr > 0.0 {
                    v
                } else {
                    0.0
                }
            }
            for i in 0..m {
                let (cr, cu, cp) = (r1[i], u1[i], p1[i]);
                let dr = lim(cr - r0[i], r2[i] - cr);
                let du = lim(cu - u0[i], u2[i] - cu);
                let dp = lim(cp - p0[i], p2[i] - cp);
                let r_ = cr - k * (cu * dr + cr * du);
                let u_ = cu - k * (cu * du + dp * ir1[i]);
                let p_ = cp - k * (cu * dp + g * cp * du);
                let (a_r, a_p) = (r_ - 0.5 * dr, p_ - 0.5 * dp);
                let (b_r, b_p) = (r_ + 0.5 * dr, p_ + 0.5 * dp);
                let ok = (a_r > 0.0) & (a_p > 0.0) & (b_r > 0.0) & (b_p > 0.0);
                lr[i] = if ok { a_r } else { cr };
                lu[i] = if ok { u_ - 0.5 * du } else { cu };
                lp[i] = if ok { a_p } else { cp };
                rr[i] = if ok { b_r } else { cr };
                ru[i] = if ok { u_ + 0.5 * du } else { cu };
                rp[i] = if ok { b_p } else { cp };
            }
            for &i in &[0, n - 1] {
                sc.lr[i] = sc.pr[i];
                sc.lu[i] = sc.pu[i];
                sc.lp[i] = sc.pp[i];
                sc.rr[i] = sc.pr[i];
                sc.ru[i] = sc.pu[i];
                sc.rp[i] = sc.pp[i];
            }
        }
        {
            let m = n - 1;
            let (lr, lu, lp) = (&sc.rr[..m], &sc.ru[..m], &sc.rp[..m]);
            let (rr, ru, rp) = (&sc.lr[1..m + 1], &sc.lu[1..m + 1], &sc.lp[1..m + 1]);
            let (f0, f1, f2, sp) = (&mut sc.f0[1..m + 1], &mut sc.f1[1..m + 1], &mut sc.f2[1..m + 1], &mut sc.sp[1..m + 1]);
            let kq = ((GAMMA + 1.0) / (2.0 * GAMMA)) as f32;
            let inv_gm1 = (1.0 / GM1) as f32;
            for i in 0..m {
                let (dl, ul, pl) = (lr[i], lu[i], lp[i]);
                let (dr, ur, pr) = (rr[i], ru[i], rp[i]);
                let (idl, idr) = (1.0 / dl, 1.0 / dr);
                let (ipl, ipr) = (1.0 / pl, 1.0 / pr);
                let al = (g * pl * idl).sqrt();
                let ar = (g * pr * idr).sqrt();
                let ps = (0.5 * (pl + pr) - 0.125 * (ur - ul) * (dl + dr) * (al + ar)).max(0.0);
                let ql = (1.0 + kq * (ps * ipl - 1.0)).max(1.0).sqrt();
                let qr = (1.0 + kq * (ps * ipr - 1.0)).max(1.0).sqrt();
                let sl = ul - al * ql;
                let sr = ur + ar * qr;
                let ml = dl * (sl - ul);
                let mr = dr * (sr - ur);
                let ss = (pr - pl + ul * ml - ur * mr) / (ml - mr);
                let el = pl * inv_gm1 + 0.5 * dl * ul * ul;
                let er = pr * inv_gm1 + 0.5 * dr * ur * ur;
                let left = ss >= 0.0;
                let d = if left { dl } else { dr };
                let u = if left { ul } else { ur };
                let pq = if left { pl } else { pr };
                let e = if left { el } else { er };
                let mm = if left { ml } else { mr };
                let s = if left { sl } else { sr };
                let id = if left { idl } else { idr };
                let scv = if left { sl.min(0.0) } else { sr.max(0.0) };
                let c = mm / (s - ss);
                f0[i] = d * u + scv * (c - d);
                f1[i] = d * u * u + pq + scv * (c * ss - d * u);
                f2[i] = u * (e + pq) + scv * (c * (e * id + (ss - u) * (ss + pq / mm)) - e);
                sp[i] = (-sl).max(sr);
            }
        }
        for i in 1..n {
            self.f0[i] = sc.f0[i] as f64;
            self.f1[i] = sc.f1[i] as f64;
            self.f2[i] = sc.f2[i] as f64;
            self.sp[i] = sc.sp[i] as f64;
        }
    }
}

/// f32 scratch for `Pipe::recon_flux_f32`.
struct Scratch32 {
    pr: Vec<f32>,
    pu: Vec<f32>,
    pp: Vec<f32>,
    ir: Vec<f32>,
    lr: Vec<f32>,
    lu: Vec<f32>,
    lp: Vec<f32>,
    rr: Vec<f32>,
    ru: Vec<f32>,
    rp: Vec<f32>,
    f0: Vec<f32>,
    f1: Vec<f32>,
    f2: Vec<f32>,
    sp: Vec<f32>,
}

impl Scratch32 {
    fn new(n: usize) -> Self {
        let v = |k| vec![0.0f32; k];
        Scratch32 { pr: v(n), pu: v(n), pp: v(n), ir: v(n), lr: v(n), lu: v(n), lp: v(n), rr: v(n), ru: v(n), rp: v(n), f0: v(n + 1), f1: v(n + 1), f2: v(n + 1), sp: v(n + 1) }
    }
}
